use std::{collections::HashMap, sync::Arc, time::Instant};

use crust_types::{AgentEvent, AgentState, CoreKind, ScopedAgent, ScopedAgentStatus, Session};
use openrouter_rs::{Content, api::chat::Message, types::Role};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::tui::log_entry_to_wrapped_lines;
use crust_core::{
    SettingsManager, append_delta, context, core_profile, format_current_session_title,
    session::SessionManager,
};

#[derive(Debug)]
pub struct App {
    pub(crate) system_message: String,
    pub(crate) core_kind: CoreKind,
    pub(crate) sessionmanager: Arc<Mutex<SessionManager>>,
    pub(crate) current_session_id: String,
    pub(crate) current_session_title: String,
    pub(crate) inputstate: InputState,
    pub(crate) agentstate: AgentState,
    pub(crate) inputbuffer: String,
    pub(crate) cursor_pos: usize,
    pub(crate) input_history: Vec<String>,
    pub(crate) input_history_index: Option<usize>,
    pub(crate) input_history_draft: String,
    pub(crate) eventlog: Vec<LogEntry>,
    pub(crate) active_assistant_log_index: Option<usize>,
    pub(crate) active_thinking_log_index: Option<usize>,
    pub(crate) event_scroll: usize,
    pub(crate) event_max_scroll: usize,
    pub(crate) cursor_visible: bool,
    pub(crate) last_cursor_toggle: Instant,
    pub(crate) agent_task: Option<JoinHandle<()>>,
    pub(crate) scoped_agent_tasks: HashMap<String, JoinHandle<()>>,
    pub(crate) scoped_agents: Vec<ScopedAgent>,
    pub(crate) sidebar_width: u16,
    pub(crate) right_pane_width: u16,
    pub(crate) is_dragging_divider: bool,
    pub(crate) is_dragging_right_divider: bool,
    pub(crate) hover_divider: bool,
    pub(crate) hover_right_divider: bool,
    pub(crate) follow_mode: bool,
    pub(crate) focus: Focus,
    pub(crate) sidebar_mode: SidebarMode,
    pub(crate) sidebar_scroll: u16,
    pub(crate) sidebar_selected: usize,
    pub(crate) slash_command_selected: usize,
}

#[derive(Debug, Clone)]
pub enum LogEntry {
    User(String),
    Assistant(String),
    AssistantFinal(String),
    Thinking { kind: String, text: String },
    ToolCall { name: String, args: String },
    ToolResult { name: String, result: String },
    System(String),
    Error(String),
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
#[allow(dead_code)]
pub enum InputState {
    Settings,
    #[default]
    FieldInput,
    Exit,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum SidebarMode {
    #[default]
    Sessions,
    Models,
    SlashCommands,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum Focus {
    #[default]
    Input,
    Sidebar,
}

fn session_input_history(session: &Session) -> Vec<String> {
    session
        .messages
        .iter()
        .filter_map(|message| {
            if message.role != Role::User {
                return None;
            }

            match &message.content {
                Content::Text(text) => {
                    let prompt = text.trim();
                    if prompt.is_empty() || prompt.starts_with('/') {
                        None
                    } else {
                        Some(prompt.to_string())
                    }
                }
                _ => None,
            }
        })
        .collect()
}

impl App {
    pub fn new() -> Self {
        let settings = SettingsManager::new().load().unwrap_or_default();
        let core_kind = settings.default_core;
        let system_message = core_profile(core_kind).system_prompt;
        let mut session_manager = SessionManager::new();

        let mut eventlog = Vec::new();
        let current_session_title;
        let current_session_id;
        let input_history;
        let initial_scroll;
        if session_manager.load_most_recent_session() {
            let current_session = session_manager.get_current_session().unwrap();
            current_session_title = format_current_session_title(current_session);
            current_session_id = current_session.id.clone();
            input_history = session_input_history(current_session);

            for message in &current_session.messages {
                if let Some(log_entry) = message_to_log_entry(message) {
                    eventlog.push(log_entry);
                }
            }

            eventlog.push(LogEntry::System(format!(
                "Loaded session: {} ({} messages)",
                current_session.name,
                current_session.messages.len()
            )));
            initial_scroll = usize::MAX;
        } else {
            let default_session = session_manager.create_session_with_core(
                "Default".to_string(),
                &system_message,
                core_kind,
            );
            current_session_title = format_current_session_title(default_session);
            current_session_id = default_session.id.clone();
            eventlog.push(LogEntry::System(format!(
                "Created session: {}",
                default_session.name
            )));
            initial_scroll = 0;
            input_history = session_input_history(default_session);
        }

        Self {
            system_message,
            core_kind,
            sessionmanager: Arc::new(Mutex::new(session_manager)),
            current_session_id,
            current_session_title,
            inputstate: InputState::default(),
            agentstate: AgentState::default(),
            inputbuffer: "".to_string(),
            cursor_pos: 0,
            input_history,
            input_history_index: None,
            input_history_draft: String::new(),
            eventlog,
            active_assistant_log_index: None,
            active_thinking_log_index: None,
            event_scroll: initial_scroll,
            event_max_scroll: 0,
            cursor_visible: true,
            last_cursor_toggle: Instant::now(),
            agent_task: None,
            scoped_agent_tasks: HashMap::new(),
            scoped_agents: Vec::new(),
            sidebar_width: 35,
            right_pane_width: 36,
            is_dragging_divider: false,
            is_dragging_right_divider: false,
            hover_divider: false,
            hover_right_divider: false,
            follow_mode: true,
            focus: Focus::default(),
            sidebar_mode: SidebarMode::default(),
            sidebar_scroll: 0,
            sidebar_selected: 0,
            slash_command_selected: 0,
        }
    }

    #[allow(dead_code)]
    pub fn get_inputstate(&self) -> InputState {
        self.inputstate.clone()
    }

    #[allow(dead_code)]
    pub fn get_agentstate(&self) -> AgentState {
        self.agentstate.clone()
    }

    pub(crate) fn reset_input_history_navigation(&mut self) {
        self.input_history_index = None;
        self.input_history_draft.clear();
    }

    pub(crate) async fn sync_input_history_from_current_session(&mut self) {
        let history = {
            let sessionmanager = self.sessionmanager.lock().await;
            sessionmanager
                .get_current_session()
                .map(session_input_history)
                .unwrap_or_default()
        };
        self.input_history = history;
        self.reset_input_history_navigation();
    }

    pub(crate) fn append_input_history(&mut self, prompt: &str) {
        let prompt = prompt.trim();
        if !prompt.is_empty() && !prompt.starts_with('/') {
            self.input_history.push(prompt.to_string());
        }
        self.reset_input_history_navigation();
    }

    pub(crate) fn retain_input_history_draft(&mut self) {
        self.input_history_draft = self.inputbuffer.clone();
        self.input_history_index = None;
    }

    pub(crate) fn load_previous_input_history(&mut self) {
        if self.input_history.is_empty() {
            return;
        }

        if self.input_history_index.is_none() {
            self.input_history_draft = self.inputbuffer.clone();
            self.input_history_index = Some(self.input_history.len().saturating_sub(1));
        } else if let Some(index) = self.input_history_index {
            if index > 0 {
                self.input_history_index = Some(index - 1);
            }
        }

        if let Some(index) = self.input_history_index {
            if let Some(entry) = self.input_history.get(index) {
                self.inputbuffer = entry.clone();
                self.cursor_pos = self.inputbuffer.len();
            }
        }
    }

    pub(crate) fn load_next_input_history(&mut self) {
        let Some(index) = self.input_history_index else {
            return;
        };

        if let Some(next_index) = index
            .checked_add(1)
            .filter(|next| *next < self.input_history.len())
        {
            self.input_history_index = Some(next_index);
            if let Some(entry) = self.input_history.get(next_index) {
                self.inputbuffer = entry.clone();
                self.cursor_pos = self.inputbuffer.len();
            }
        } else {
            self.inputbuffer = self.input_history_draft.clone();
            self.cursor_pos = self.inputbuffer.len();
            self.reset_input_history_navigation();
        }
    }

    pub(crate) fn find_scoped_agent_index(&self, name_or_id: &str) -> Option<usize> {
        self.scoped_agents.iter().position(|agent| {
            agent.id == name_or_id || agent.name.eq_ignore_ascii_case(name_or_id.trim())
        })
    }

    pub(crate) fn push_scoped_agent_event(&mut self, id: &str, message: String) {
        if let Some(index) = self.find_scoped_agent_index(id) {
            let agent = &mut self.scoped_agents[index];
            agent.events.push(message);
            if agent.events.len() > 20 {
                let overflow = agent.events.len().saturating_sub(20);
                agent.events.drain(0..overflow);
            }
        }
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::UserSubmitted { prompt } => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.eventlog.push(LogEntry::User(prompt));
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::Thinking { kind, text } => {
                match self.active_thinking_log_index {
                    Some(index) => match self.eventlog.get_mut(index) {
                        Some(LogEntry::Thinking {
                            text: existing_text,
                            ..
                        }) => append_delta(existing_text, &text),
                        _ => {
                            self.eventlog.push(LogEntry::Thinking { kind, text });
                            self.active_thinking_log_index = Some(self.eventlog.len() - 1);
                        }
                    },
                    None => {
                        self.eventlog.push(LogEntry::Thinking { kind, text });
                        self.active_thinking_log_index = Some(self.eventlog.len() - 1);
                    }
                }
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::ToolCallStarted { name, args } => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.eventlog.push(LogEntry::ToolCall { name, args });
                self.agentstate = AgentState::Tool;
            }
            AgentEvent::ToolCallFinished { name, result } => {
                self.eventlog.push(LogEntry::ToolResult { name, result });
                self.agentstate = AgentState::Tool;
            }
            AgentEvent::AssistantDelta { text } => {
                match self.active_assistant_log_index {
                    Some(index) => match self.eventlog.get_mut(index) {
                        Some(LogEntry::Assistant(existing)) => append_delta(existing, &text),
                        _ => {
                            self.eventlog.push(LogEntry::Assistant(text));
                            self.active_assistant_log_index = Some(self.eventlog.len() - 1);
                        }
                    },
                    None => {
                        self.eventlog.push(LogEntry::Assistant(text));
                        self.active_assistant_log_index = Some(self.eventlog.len() - 1);
                    }
                }
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::AssistantFinal { text } => {
                let already_displayed = self
                    .active_assistant_log_index
                    .and_then(|index| self.eventlog.get(index))
                    .is_some_and(
                        |entry| matches!(entry, LogEntry::Assistant(existing) if existing == &text),
                    );

                if already_displayed {
                    if let Some(index) = self.active_assistant_log_index {
                        self.eventlog[index] = LogEntry::AssistantFinal(text);
                    }
                } else {
                    self.eventlog.push(LogEntry::AssistantFinal(text));
                }
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.agentstate = AgentState::Done;
            }
            AgentEvent::Error { message } => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.eventlog.push(LogEntry::Error(message));
                self.agentstate = AgentState::Error;
            }
            AgentEvent::SystemNotice { message } => {
                self.eventlog.push(LogEntry::System(message));
            }
            AgentEvent::MaxStepsReached => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.eventlog.push(LogEntry::Error(
                    "Agent reached max steps without producing a final response.".to_string(),
                ));
                self.agentstate = AgentState::Done;
            }
            AgentEvent::SessionTitleUpdated { title } => {
                self.current_session_title = title;
            }
            AgentEvent::ScopedAgentStarted {
                id,
                name,
                task,
                max_steps,
            } => {
                self.scoped_agents.push(ScopedAgent {
                    id: id.clone(),
                    name,
                    task,
                    status: ScopedAgentStatus::Running,
                    current_step: 0,
                    max_steps,
                    events: vec!["started".to_string()],
                    result: None,
                });
                self.eventlog
                    .push(LogEntry::System(format!("Scoped agent `{id}` started.")));
            }
            AgentEvent::ScopedAgentStep { id, step } => {
                if let Some(index) = self.find_scoped_agent_index(&id) {
                    self.scoped_agents[index].current_step = step;
                    self.scoped_agents[index].status = ScopedAgentStatus::Running;
                }
                self.push_scoped_agent_event(&id, format!("step {step}"));
            }
            AgentEvent::ScopedAgentStatus { id, status } => {
                if let Some(index) = self.find_scoped_agent_index(&id) {
                    self.scoped_agents[index].status = status.clone();
                }
                self.push_scoped_agent_event(&id, status.to_string());
            }
            AgentEvent::ScopedAgentLog { id, message } => {
                self.push_scoped_agent_event(&id, message);
            }
            AgentEvent::ScopedAgentFinished { id, result } => {
                if let Some(index) = self.find_scoped_agent_index(&id) {
                    self.scoped_agents[index].status = ScopedAgentStatus::Done;
                    self.scoped_agents[index].result = Some(result.clone());
                }
                self.scoped_agent_tasks.remove(&id);
                self.push_scoped_agent_event(&id, "finished".to_string());
                self.eventlog.push(LogEntry::System(format!(
                    "Scoped agent `{id}` completed:\n{}",
                    context::truncate_middle(&result, 4_000)
                )));
            }
            AgentEvent::ScopedAgentError { id, message } => {
                if let Some(index) = self.find_scoped_agent_index(&id) {
                    self.scoped_agents[index].status = ScopedAgentStatus::Error;
                }
                self.scoped_agent_tasks.remove(&id);
                self.push_scoped_agent_event(&id, format!("error: {message}"));
                self.eventlog.push(LogEntry::Error(format!(
                    "Scoped agent `{id}` failed: {message}"
                )));
            }
            AgentEvent::Finished => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
                self.agent_task = None;
                self.agentstate = AgentState::Done;
            }
        }
    }

    #[allow(dead_code)]
    pub fn recalculate_scroll_bounds(&mut self, viewport_height: u16, viewport_width: u16) {
        let events_view_height = usize::from(viewport_height.saturating_sub(2));
        let events_view_width = usize::from(viewport_width.saturating_sub(2)).max(1);

        let rendered_height: usize = self
            .eventlog
            .iter()
            .map(|entry| log_entry_to_wrapped_lines(entry, events_view_width).len())
            .sum();

        let max_scroll = rendered_height.saturating_sub(events_view_height);
        self.event_max_scroll = max_scroll;
        self.event_scroll = self.event_scroll.min(max_scroll);
    }
}

pub(crate) fn message_to_log_entry(message: &Message) -> Option<LogEntry> {
    match message.role {
        Role::User => {
            if let Content::Text(text) = &message.content {
                Some(LogEntry::User(text.clone()))
            } else {
                None
            }
        }
        Role::Assistant => {
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    let tc = &tool_calls[0];
                    Some(LogEntry::ToolCall {
                        name: tc.name().to_string(),
                        args: tc.arguments_json().to_string(),
                    })
                } else if let Content::Text(text) = &message.content {
                    Some(LogEntry::AssistantFinal(text.clone()))
                } else {
                    None
                }
            } else if let Content::Text(text) = &message.content {
                Some(LogEntry::AssistantFinal(text.clone()))
            } else {
                None
            }
        }
        Role::Tool => {
            if let Content::Text(text) = &message.content {
                let name = message
                    .name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                Some(LogEntry::ToolResult {
                    name,
                    result: text.clone(),
                })
            } else {
                None
            }
        }
        Role::System => None,
        _ => None,
    }
}
