use std::{error::Error, sync::Arc, time::Duration};

use crust_types::{
    AgentEvent, EditFileParams, ReadFileParams, ScopedAgent, ScopedAgentStatus, ShellParams,
    TaggedAgentEvent,
};
use futures_util::StreamExt;
use openrouter_rs::{
    api::chat::Message,
    types::{Role, ToolCall, stream::StreamEvent, typed_tool::TypedTool},
};
use tokio::sync::{Mutex, mpsc};
use tokio::time;

use crate::{
    context,
    session::SessionManager,
    tools::execute_tool_call,
    util::{append_delta, compact_tool_call_text, compact_tool_result_text},
};

pub fn parse_scoped_agent_command(prompt: &str) -> Option<(String, String)> {
    let args = prompt.strip_prefix("/agent ")?.trim();
    let mut parts = args.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let task = parts.next().unwrap_or("").trim();
    if name.is_empty() || task.is_empty() {
        None
    } else {
        Some((name.to_string(), task.to_string()))
    }
}

pub fn format_scoped_agents(agents: &[ScopedAgent]) -> String {
    if agents.is_empty() {
        return "No scoped agents in this session.".to_string();
    }

    agents
        .iter()
        .map(|agent| {
            format!(
                "{} [{}] {}/{} - {}",
                agent.name,
                agent.status,
                agent.current_step,
                agent.max_steps,
                context::truncate_middle(&agent.task, 160)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn build_scoped_agent_context(
    sessionmanager: &Arc<Mutex<SessionManager>>,
    task: &str,
) -> Result<(Vec<Message>, crust_types::Config), Box<dyn Error + Send + Sync>> {
    let (session, config) = {
        let sm = sessionmanager.lock().await;
        let session = sm
            .get_current_session()
            .ok_or_else(|| std::io::Error::other("No current session."))?
            .clone();
        let config = session.get_config();
        (session, config)
    };

    let mut messages = vec![Message::new(
        Role::System,
        "You are a scoped child coding agent inside a parent Crust session. Work only on the explicit task. Keep context and responses short. You may use only the tools provided to this scoped run. Do not invoke, request, simulate, or spawn another agent; nested agents are forbidden. Do not auto-route work to workflows or agents. Return a concise final result with files changed and validation run.",
    )];

    if let Some(summary) = session
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
    {
        messages.push(Message::new(
            Role::System,
            format!(
                "Parent session compacted summary for background only:\n{}",
                context::truncate_middle(summary, 4_000)
            ),
        ));
    }

    let recent_non_system: Vec<Message> = session
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
        .rev()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| context::truncate_message_text(&message, 4_000))
        .collect();
    messages.extend(recent_non_system);

    messages.push(Message::new(Role::User, format!("Scoped task:\n{task}")));

    Ok((messages, config))
}

fn nested_agent_attempt(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("/agent")
        || lower.contains("crust agent")
        || lower.contains("crust /agent")
        || lower.contains("spawn agent")
}

async fn execute_scoped_tool_call(tc: &ToolCall) -> Result<String, Box<dyn Error + Send + Sync>> {
    if tc.is_tool::<ShellParams>() {
        let params = tc.parse_params::<ShellParams>()?;
        if nested_agent_attempt(&params.command) {
            return Err("scoped agents cannot spawn or invoke nested agents".into());
        }
        return execute_tool_call(tc).await;
    }

    if tc.is_tool::<ReadFileParams>() || tc.is_tool::<EditFileParams>() {
        return execute_tool_call(tc).await;
    }

    Err(format!("tool `{}` is not available to scoped agents", tc.name()).into())
}

pub async fn scoped_agent_run(
    sessionmanager: Arc<Mutex<SessionManager>>,
    task: String,
    scoped_agent_id: String,
    parent_session_id: String,
    max_steps: u32,
    event_tx: mpsc::Sender<TaggedAgentEvent>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut messages, config) = build_scoped_agent_context(&sessionmanager, &task).await?;
    let client = config.client_builder()?;

    for step in 0..max_steps {
        let step_number = step + 1;
        let _ = event_tx
            .send(TaggedAgentEvent {
                session_id: parent_session_id.clone(),
                event: AgentEvent::ScopedAgentStep {
                    id: scoped_agent_id.clone(),
                    step: step_number,
                },
            })
            .await;

        let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
            .model(&config.modelname)
            .messages(messages.clone())
            .typed_tools_batch(&[
                ShellParams::create_tool(),
                ReadFileParams::create_tool(),
                EditFileParams::create_tool(),
            ])
            .temperature(0.2f64)
            .build()?;

        let mut stream = client.chat().stream_tool_aware(&request).await?;
        let mut final_content = String::new();
        let mut final_tool_calls = Vec::new();
        let mut saw_done = false;
        const SCOPED_STREAM_EVENT_TIMEOUT: Duration = Duration::from_secs(120);

        loop {
            let event = match time::timeout(SCOPED_STREAM_EVENT_TIMEOUT, stream.next()).await {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(_) => return Err("scoped agent OpenRouter stream timed out".into()),
            };

            match event {
                StreamEvent::ContentDelta(text) => {
                    append_delta(&mut final_content, &text);
                }
                StreamEvent::ReasoningDelta(_) | StreamEvent::ReasoningDetailsDelta(_) => {}
                StreamEvent::Done { tool_calls, .. } => {
                    saw_done = true;
                    final_tool_calls = tool_calls;
                }
                StreamEvent::Error(err) => {
                    return Err(Box::new(err) as Box<dyn Error + Send + Sync>);
                }
            }
        }

        if !saw_done {
            return Err("scoped agent stream ended without a final event".into());
        }

        if final_tool_calls.is_empty() {
            let result = if final_content.trim().is_empty() {
                "Scoped agent finished without text output.".to_string()
            } else {
                final_content
            };
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_session(
                    &parent_session_id,
                    Message::new(
                        Role::Assistant,
                        format!(
                            "Scoped agent `{}` final result:\n{}",
                            scoped_agent_id,
                            context::truncate_middle(&result, 8_000)
                        ),
                    ),
                );
            }
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: parent_session_id,
                    event: AgentEvent::ScopedAgentFinished {
                        id: scoped_agent_id,
                        result,
                    },
                })
                .await;
            return Ok(());
        }

        messages.push(Message::assistant_with_tool_calls(
            final_content.as_str(),
            final_tool_calls.clone(),
        ));

        for tool_call in final_tool_calls {
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: parent_session_id.clone(),
                    event: AgentEvent::ScopedAgentStatus {
                        id: scoped_agent_id.clone(),
                        status: ScopedAgentStatus::Tool,
                    },
                })
                .await;
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: parent_session_id.clone(),
                    event: AgentEvent::ScopedAgentLog {
                        id: scoped_agent_id.clone(),
                        message: compact_tool_call_text(
                            tool_call.name(),
                            tool_call.arguments_json(),
                        ),
                    },
                })
                .await;

            let tool_result = match execute_scoped_tool_call(&tool_call).await {
                Ok(result) => result,
                Err(err) => format!("Tool `{}` failed: {err}", tool_call.name()),
            };
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: parent_session_id.clone(),
                    event: AgentEvent::ScopedAgentLog {
                        id: scoped_agent_id.clone(),
                        message: compact_tool_result_text(tool_call.name(), &tool_result),
                    },
                })
                .await;
            messages.push(Message::tool_response_named(
                tool_call.id(),
                tool_call.name(),
                context::truncate_middle(&tool_result, 8_000),
            ));
        }
    }

    Err(format!("scoped agent reached max steps ({max_steps}) without a final response").into())
}
