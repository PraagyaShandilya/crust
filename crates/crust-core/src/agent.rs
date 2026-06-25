use std::{error::Error, sync::Arc, time::Duration};

use crust_types::{
    AgentEvent, EditFileParams, LangGraphRunParams, ReadFileParams, ShellParams, TaggedAgentEvent,
    WebSearchParams, WriteFileParams,
};
use futures_util::StreamExt;
use openrouter_rs::{
    Content,
    api::chat::Message,
    types::{Role, stream::StreamEvent, typed_tool::TypedTool},
};
use tokio::sync::{Mutex, mpsc};
use tokio::time;

use crate::{
    ContextBuilder,
    cores::core_profile,
    langgraph::load_langgraph_registry,
    session::SessionManager,
    tools::execute_tool_call,
    util::{
        append_delta, context_pressure_message, format_current_session_title, format_token_count,
    },
};

async fn emit_session_title(
    sessionmanager: &Arc<Mutex<SessionManager>>,
    session_id: &str,
    event_tx: &mpsc::Sender<TaggedAgentEvent>,
) {
    let title = {
        let sm = sessionmanager.lock().await;
        sm.get_current_session().map(format_current_session_title)
    };
    if let Some(title) = title {
        let _ = event_tx
            .send(TaggedAgentEvent {
                session_id: session_id.to_string(),
                event: AgentEvent::SessionTitleUpdated { title },
            })
            .await;
    }
}

pub async fn agent_main_run(
    sessionmanager: Arc<Mutex<SessionManager>>,
    prompt: String,
    session_id: String,
    event_tx: mpsc::Sender<TaggedAgentEvent>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let user_message = Message {
        role: Role::User,
        content: Content::Text(prompt),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    };
    {
        let mut sm = sessionmanager.lock().await;
        sm.add_message_to_current(user_message);
    }
    emit_session_title(&sessionmanager, &session_id, &event_tx).await;

    let mut config = {
        let sm = sessionmanager.lock().await;
        sm.get_current_config().unwrap()
    };
    let core = {
        let sm = sessionmanager.lock().await;
        sm.get_current_session()
            .map(|session| core_profile(session.core_profile))
            .unwrap_or_else(|| core_profile(Default::default()))
    };
    if let Some(default_model) = core.default_model {
        config.modelname = default_model.to_string();
    }

    let client = config.client_builder()?;
    let context_builder = ContextBuilder::default();

    for _step in 0..config.max_agent_steps {
        let (current_messages, estimated_context_tokens, estimated_context_ratio) = {
            let sm = sessionmanager.lock().await;
            let session = sm.get_current_session().unwrap();
            let context = context_builder.build_context(session, &config);
            let estimated_tokens = context_builder.estimate_context_tokens(&context);
            let estimated_ratio = context_builder.estimated_context_ratio(&context, &config);
            (context, estimated_tokens, estimated_ratio)
        };

        if context_builder.should_compact_estimated_context(&current_messages, &config) {
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: session_id.clone(),
                    event: AgentEvent::SystemNotice {
                        message: format!(
                            "Estimated context before request is {}/{} toks ({:.1}%). Run /compact if the next request is slow or fails from context size.",
                            format_token_count(estimated_context_tokens),
                            format_token_count(context_builder.context_window_tokens(&config)),
                            estimated_context_ratio * 100.0,
                        ),
                    },
                })
                .await;
        }

        let mut tools = vec![
            WebSearchParams::create_tool(),
            ShellParams::create_tool(),
            ReadFileParams::create_tool(),
            EditFileParams::create_tool(),
            WriteFileParams::create_tool(),
        ];
        if load_langgraph_registry()
            .map(|registry| !registry.servers.is_empty())
            .unwrap_or(false)
        {
            tools.push(LangGraphRunParams::create_tool());
        }

        let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
            .model(&config.modelname)
            .messages(current_messages)
            .typed_tools_batch(&tools)
            .temperature(0.2f64)
            .build()?;

        let mut stream = match client.chat().stream_tool_aware(&request).await {
            Ok(stream) => stream,
            Err(err) => {
                let modelname = config.modelname.clone();
                let _ = event_tx
                    .send(TaggedAgentEvent {
                        session_id: session_id.clone(),
                        event: AgentEvent::Error {
                            message: format!(
                                "OpenRouter streaming request failed for model `{modelname}`: {err:?}"
                            ),
                        },
                    })
                    .await;
                return Err(Box::new(err) as Box<dyn Error + Send + Sync>);
            }
        };

        let mut final_content = String::new();
        let mut final_tool_calls = Vec::new();
        let mut saw_done = false;
        let mut saw_reasoning_delta = false;

        const STREAM_EVENT_TIMEOUT: Duration = Duration::from_secs(180);
        loop {
            let event = match time::timeout(STREAM_EVENT_TIMEOUT, stream.next()).await {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(_) => {
                    let _ = event_tx
                        .send(TaggedAgentEvent {
                            session_id: session_id.clone(),
                            event: AgentEvent::Error {
                                message: format!(
                                    "OpenRouter stream timed out after {} seconds without a new event.",
                                    STREAM_EVENT_TIMEOUT.as_secs()
                                ),
                            },
                        })
                        .await;
                    return Err("OpenRouter stream timed out".into());
                }
            };

            match event {
                StreamEvent::ContentDelta(text) => {
                    append_delta(&mut final_content, &text);
                    let _ = event_tx
                        .send(TaggedAgentEvent {
                            session_id: session_id.clone(),
                            event: AgentEvent::AssistantDelta { text },
                        })
                        .await;
                }
                StreamEvent::ReasoningDelta(text) => {
                    saw_reasoning_delta = true;
                    let _ = event_tx
                        .send(TaggedAgentEvent {
                            session_id: session_id.clone(),
                            event: AgentEvent::Thinking {
                                kind: "reasoning".to_string(),
                                text,
                            },
                        })
                        .await;
                }
                StreamEvent::ReasoningDetailsDelta(details) => {
                    if saw_reasoning_delta {
                        continue;
                    }

                    for block in details {
                        if let Some(text) = block.content() {
                            let _ = event_tx
                                .send(TaggedAgentEvent {
                                    session_id: session_id.clone(),
                                    event: AgentEvent::Thinking {
                                        kind: "reasoning".to_string(),
                                        text: text.to_string(),
                                    },
                                })
                                .await;
                        }
                    }
                }
                StreamEvent::Done {
                    tool_calls, usage, ..
                } => {
                    saw_done = true;
                    final_tool_calls = tool_calls;

                    if let Some(usage) = usage {
                        let prompt_tokens_sent = usage.prompt_tokens;
                        {
                            let mut sm = sessionmanager.lock().await;
                            sm.record_usage_to_current(
                                usage.prompt_tokens,
                                usage.completion_tokens,
                                usage.total_tokens,
                            );
                        }
                        emit_session_title(&sessionmanager, &session_id, &event_tx).await;

                        if let Some(message) =
                            context_pressure_message(&context_builder, &config, prompt_tokens_sent)
                        {
                            let _ = event_tx
                                .send(TaggedAgentEvent {
                                    session_id: session_id.clone(),
                                    event: AgentEvent::SystemNotice { message },
                                })
                                .await;
                        }
                    }
                }
                StreamEvent::Error(err) => {
                    let _ = event_tx
                        .send(TaggedAgentEvent {
                            session_id: session_id.clone(),
                            event: AgentEvent::Error {
                                message: format!("OpenRouter stream error: {err:?}"),
                            },
                        })
                        .await;
                    return Err(Box::new(err) as Box<dyn Error + Send + Sync>);
                }
            }
        }

        if !saw_done {
            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: session_id.clone(),
                    event: AgentEvent::Error {
                        message: "Crust Agent: stream ended without a final event.".to_string(),
                    },
                })
                .await;
            break;
        }

        if !final_tool_calls.is_empty() {
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_current(Message::assistant_with_tool_calls(
                    final_content.as_str(),
                    final_tool_calls.clone(),
                ));
            }
            emit_session_title(&sessionmanager, &session_id, &event_tx).await;

            for tool_call in &final_tool_calls {
                let _ = event_tx
                    .send(TaggedAgentEvent {
                        session_id: session_id.clone(),
                        event: AgentEvent::ToolCallStarted {
                            name: tool_call.name().to_string(),
                            args: tool_call.arguments_json().to_string(),
                        },
                    })
                    .await;

                let tool_result = match execute_tool_call(tool_call).await {
                    Ok(result) => result,
                    Err(err) => {
                        let error_msg = format!("Tool `{}` failed: {err}", tool_call.name());
                        let _ = event_tx
                            .send(TaggedAgentEvent {
                                session_id: session_id.clone(),
                                event: AgentEvent::Error {
                                    message: error_msg.clone(),
                                },
                            })
                            .await;
                        {
                            let mut sm = sessionmanager.lock().await;
                            sm.add_message_to_current(Message::tool_response_named(
                                tool_call.id(),
                                tool_call.name(),
                                error_msg.clone(),
                            ));
                        }
                        emit_session_title(&sessionmanager, &session_id, &event_tx).await;
                        continue;
                    }
                };

                let _ = event_tx
                    .send(TaggedAgentEvent {
                        session_id: session_id.clone(),
                        event: AgentEvent::ToolCallFinished {
                            name: tool_call.name().to_string(),
                            result: tool_result.clone(),
                        },
                    })
                    .await;

                {
                    let mut sm = sessionmanager.lock().await;
                    sm.add_message_to_current(Message::tool_response_named(
                        tool_call.id(),
                        tool_call.name(),
                        tool_result,
                    ));
                }
                emit_session_title(&sessionmanager, &session_id, &event_tx).await;
            }

            continue;
        } else {
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_current(Message::new(Role::Assistant, final_content.clone()));
            }
            emit_session_title(&sessionmanager, &session_id, &event_tx).await;

            let _ = event_tx
                .send(TaggedAgentEvent {
                    session_id: session_id.clone(),
                    event: AgentEvent::AssistantFinal {
                        text: final_content,
                    },
                })
                .await;
            return Ok(());
        }
    }

    let _ = event_tx
        .send(TaggedAgentEvent {
            session_id: session_id.clone(),
            event: AgentEvent::MaxStepsReached,
        })
        .await;
    Ok(())
}
