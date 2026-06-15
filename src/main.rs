mod models_generated;
mod session;

use futures_util::StreamExt;
use openrouter_rs::{
    Content,
    api::chat::Message,
    types::{Role, ToolCall, stream::StreamEvent, typed_tool::TypedTool},
};
use rsbash::rashf;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use session::{Session, SessionManager};
use std::{
    error::Error,
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
use tavily::Tavily;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            MouseEventKind,
        },
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

// setup for tui
#[derive(Debug)]
pub struct App {
    system_message: &'static str,
    sessionmanager: Arc<Mutex<SessionManager>>,
    current_session_title: String,
    inputstate: InputState,
    agentstate: AgentState,
    inputbuffer: String,
    eventlog: Vec<LogEntry>,
    event_scroll: usize,
    event_max_scroll: usize,
    cursor_visible: bool,
    last_cursor_toggle: Instant,
    agent_task: Option<JoinHandle<()>>,
    sidebar_width: u16,
    is_dragging_divider: bool,
    hover_divider: bool,
    follow_mode: bool,
    focus: Focus,
    sidebar_mode: SidebarMode,
    sidebar_scroll: u16,
    sidebar_selected: usize,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum AgentState {
    #[default]
    Idle,
    Done,
    Thinking,
    Tool,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    UserSubmitted { prompt: String },
    Thinking { kind: String, text: String },
    ToolCallStarted { name: String, args: String },
    ToolCallFinished { name: String, result: String },
    AssistantDelta { text: String },
    AssistantFinal { text: String },
    Error { message: String },
    MaxStepsReached,
    SessionTitleUpdated { title: String },
    Finished,
}

#[derive(Debug, Clone)]
pub enum LogEntry {
    User(String),
    Assistant(String),
    Thinking { kind: String, text: String },
    ToolCall { name: String, args: String },
    ToolResult { name: String, result: String },
    System(String),
    Error(String),
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
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
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum Focus {
    #[default]
    Input,
    Sidebar,
}

fn format_token_count(tokens: usize) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

fn format_current_session_title(session: &Session) -> String {
    let used_tokens = session.latest_total_tokens as usize;
    match models_generated::get_openrouter_model(&session.config.modelname) {
        Some(model) => format!(
            "{} - {} | {} | {}/{} toks ({:.1}%) | total {} toks",
            session.name,
            session.id,
            model.id,
            format_token_count(used_tokens),
            format_token_count(model.context_window as usize),
            (used_tokens as f64 / model.context_window as f64) * 100.0,
            format_token_count(session.cumulative_total_tokens as usize)
        ),
        None => format!(
            "{} - {} | {} | {} toks/? ctx | total {} toks",
            session.name,
            session.id,
            session.config.modelname,
            format_token_count(used_tokens),
            format_token_count(session.cumulative_total_tokens as usize)
        ),
    }
}

impl App {
    pub fn new() -> Self {
        // Initialize messages with system prompt
        let system_message = r#"You are an AI agent given tools to be able to help people you can use the
        bash tool to run unix commands in the shell, the read write and edit tools
        respectively for editing files that you know the filenames of and the web search
        tool to interface with the web."#;

        // Initialize Session Manager
        let mut session_manager = SessionManager::new();

        let mut eventlog = Vec::new();
        let current_session_title;
        let initial_scroll;
        if session_manager.load_most_recent_session() {
            let current_session = session_manager.get_current_session().unwrap();
            current_session_title = format_current_session_title(current_session);

            // Load historical messages into eventlog
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

            // Set scroll to max value so it auto-scrolls to bottom on first draw
            initial_scroll = usize::MAX;
        } else {
            let default_session =
                session_manager.create_session("Default".to_string(), system_message);
            current_session_title = format_current_session_title(default_session);
            eventlog.push(LogEntry::System(format!(
                "Created session: {}",
                default_session.name
            )));
            initial_scroll = 0;
        }

        Self {
            system_message,
            sessionmanager: Arc::new(Mutex::new(session_manager)),
            current_session_title,
            inputstate: InputState::default(),
            agentstate: AgentState::default(),
            inputbuffer: "".to_string(),
            eventlog,
            event_scroll: initial_scroll,
            event_max_scroll: 0,
            cursor_visible: true,
            last_cursor_toggle: Instant::now(),
            agent_task: None,
            sidebar_width: 35,
            is_dragging_divider: false,
            hover_divider: false,
            follow_mode: true,
            focus: Focus::default(),
            sidebar_mode: SidebarMode::default(),
            sidebar_scroll: 0,
            sidebar_selected: 0,
        }
    }

    pub fn get_inputstate(&self) -> InputState {
        self.inputstate.clone()
    }

    pub fn get_agentstate(&self) -> AgentState {
        self.agentstate.clone()
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::UserSubmitted { prompt } => {
                self.eventlog.push(LogEntry::User(prompt));
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::Thinking { kind, text } => {
                self.eventlog.push(LogEntry::Thinking { kind, text });
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::ToolCallStarted { name, args } => {
                self.eventlog.push(LogEntry::ToolCall { name, args });
                self.agentstate = AgentState::Tool;
            }
            AgentEvent::ToolCallFinished { name, result } => {
                self.eventlog.push(LogEntry::ToolResult { name, result });
                self.agentstate = AgentState::Tool;
            }
            AgentEvent::AssistantDelta { text } => {
                match self.eventlog.last_mut() {
                    Some(LogEntry::Assistant(existing)) => existing.push_str(&text),
                    _ => self.eventlog.push(LogEntry::Assistant(text)),
                }
                self.agentstate = AgentState::Thinking;
            }
            AgentEvent::AssistantFinal { text } => {
                if !matches!(self.eventlog.last(), Some(LogEntry::Assistant(existing)) if existing == &text) {
                    self.eventlog.push(LogEntry::Assistant(text));
                }
                self.agentstate = AgentState::Done;
            }
            AgentEvent::Error { message } => {
                self.eventlog.push(LogEntry::Error(message));
                self.agentstate = AgentState::Done;
            }
            AgentEvent::MaxStepsReached => {
                self.eventlog.push(LogEntry::Error(
                    "Agent reached max steps without producing a final response.".to_string(),
                ));
                self.agentstate = AgentState::Done;
            }
            AgentEvent::SessionTitleUpdated { title } => {
                self.current_session_title = title;
            }
            AgentEvent::Finished => {
                self.agent_task = None;
                self.agentstate = AgentState::Done;
            }
        }
    }

    /// Recalculate scroll bounds based on current event log and viewport size
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

        // Clamp current scroll to new max
        self.event_scroll = self.event_scroll.min(max_scroll);
    }
}

/// Convert a stored Message to a LogEntry for display in the event pane
fn message_to_log_entry(message: &Message) -> Option<LogEntry> {
    match message.role {
        Role::User => {
            if let Content::Text(text) = &message.content {
                Some(LogEntry::User(text.clone()))
            } else {
                None
            }
        }
        Role::Assistant => {
            // Check if this is a tool call message
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    // Show the first tool call (simplified view)
                    let tc = &tool_calls[0];
                    Some(LogEntry::ToolCall {
                        name: tc.name().to_string(),
                        args: tc.arguments_json().to_string(),
                    })
                } else if let Content::Text(text) = &message.content {
                    Some(LogEntry::Assistant(text.clone()))
                } else {
                    None
                }
            } else if let Content::Text(text) = &message.content {
                Some(LogEntry::Assistant(text.clone()))
            } else {
                None
            }
        }
        Role::Tool => {
            // Tool response message
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
        Role::System => {
            // Skip system messages in the event log (they're internal)
            None
        }
        _ => None,
    }
}
// tool setup for web search
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct WebSearchParams {
    query: String,
    max_results: usize,
}

impl TypedTool for WebSearchParams {
    fn name() -> &'static str {
        "web_search_tool"
    }

    fn description() -> &'static str {
        "Look up the web for a query using search"
    }
}

// tool setup for bash commands
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct BashParams {
    timeout: u32,
    command: String,
}

impl TypedTool for BashParams {
    fn name() -> &'static str {
        "bash_calling_tool"
    }

    fn description() -> &'static str {
        "Run a predefined bash command in the shell in of the workspace"
    }
}

// tool setup for reading files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct ReadFileParams {
    filename: String,
    /// 1-indexed line number to start reading from. Omit to start at line 1.
    offset: Option<usize>,
    /// Maximum number of lines to read. Omit to read up to 2000 lines / 50KB.
    limit: Option<usize>,
}

impl TypedTool for ReadFileParams {
    fn name() -> &'static str {
        "read_file_tool"
    }
    fn description() -> &'static str {
        "Read a file by filename. For normal files, omit offset and limit to read up to 2000 lines or 50KB at once. offset is a 1-indexed line number and limit is a line count. Use offset/limit only for large files; continue from next_offset if returned."
    }
}

// tool setup for writing files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct WriteFileParams {
    filename: String,
    content: String,
}

impl TypedTool for WriteFileParams {
    fn name() -> &'static str {
        "write_file_tool"
    }
    fn description() -> &'static str {
        "Write a file using its file name and a content string"
    }
}

// tool setup for editing files
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct EditFileParams {
    filename: String,
    oldcontent: String,
    newcontent: String,
}

impl TypedTool for EditFileParams {
    fn name() -> &'static str {
        "edit_file_tool"
    }
    fn description() -> &'static str {
        "Edit a file using its file finding and editing an oldcontent and replacing it with newcontent"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // create app and run it
    let mut app = App::new();
    let _ = run_app(&mut terminal, &mut app).await?;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<bool, Box<dyn Error>>
where
    B::Error: Error + 'static,
{
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(100);

    loop {
        while let Ok(event) = agent_rx.try_recv() {
            app.handle_agent_event(event);
        }

        if app.last_cursor_toggle.elapsed() >= Duration::from_millis(500) {
            app.cursor_visible = !app.cursor_visible;
            app.last_cursor_toggle = Instant::now();
        }

        terminal.draw(|f| ui(f, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let event = event::read()?;

        match event {
            Event::Mouse(mouse_event) => {
                let divider_x = app.sidebar_width;
                let area = terminal.size()?;

                let h_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(app.sidebar_width), Constraint::Min(10)])
                    .split(area.into());

                let v_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(5)])
                    .split(h_chunks[1]);

                let events_area = v_chunks[0];
                let over_events = mouse_event.column >= events_area.x
                    && mouse_event.column < events_area.x.saturating_add(events_area.width)
                    && mouse_event.row >= events_area.y
                    && mouse_event.row < events_area.y.saturating_add(events_area.height);

                app.hover_divider =
                    mouse_event.column.abs_diff(divider_x) <= 1 && mouse_event.row < area.height;

                match mouse_event.kind {
                    MouseEventKind::ScrollUp if over_events => {
                        app.event_scroll = app.event_scroll.saturating_sub(3);
                        app.follow_mode = false;
                    }
                    MouseEventKind::ScrollDown if over_events => {
                        app.event_scroll = app.event_scroll.saturating_add(3);
                        app.follow_mode = app.event_scroll >= app.event_max_scroll;
                    }
                    MouseEventKind::Down(_) if app.hover_divider => {
                        app.is_dragging_divider = true;
                    }
                    MouseEventKind::Drag(_) if app.is_dragging_divider => {
                        let total = area.width;
                        app.sidebar_width = mouse_event.column.clamp(10, total.saturating_sub(20));
                    }
                    MouseEventKind::Up(_) => {
                        app.is_dragging_divider = false;
                    }
                    _ => {}
                }
                continue;
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global keys that work regardless of focus
                match key.code {
                    KeyCode::Esc => {
                        if let Some(task) = app.agent_task.take() {
                            task.abort();
                            app.handle_agent_event(AgentEvent::Error {
                                message: "Agent run cancelled by user.".to_string(),
                            });
                        } else {
                            break;
                        }
                        continue;
                    }
                    KeyCode::Tab => {
                        app.focus = match app.focus {
                            Focus::Input => Focus::Sidebar,
                            Focus::Sidebar => Focus::Input,
                        };
                        continue;
                    }
                    KeyCode::Char(c) if app.focus == Focus::Input => {
                        app.inputbuffer.push(c);
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Backspace if app.focus == Focus::Input => {
                        app.inputbuffer.pop();
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Enter if app.focus == Focus::Input => {
                        let prompt = app.inputbuffer.trim().to_string();
                        app.inputbuffer.clear();
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();

                        if prompt.is_empty() {
                            continue;
                        }
                        if handle_session_command(app, &prompt).await {
                            if app.inputstate == InputState::Exit {
                                break;
                            }
                            continue;
                        }

                        if app.agent_task.is_some() {
                            app.handle_agent_event(AgentEvent::Error {
                                message: "Agent is already running.".to_string(),
                            });
                            continue;
                        }

                        app.handle_agent_event(AgentEvent::UserSubmitted {
                            prompt: prompt.clone(),
                        });
                        terminal.draw(|f| ui(f, app))?;

                        let sessionmanager = Arc::clone(&app.sessionmanager);
                        let tx = agent_tx.clone();
                        app.agent_task = Some(tokio::spawn(async move {
                            if let Err(err) =
                                agent_main_run(sessionmanager, prompt, tx.clone()).await
                            {
                                let _ = tx
                                    .send(AgentEvent::Error {
                                        message: err.to_string(),
                                    })
                                    .await;
                            }
                            let _ = tx.send(AgentEvent::Finished).await;
                        }));
                        continue;
                    }
                    _ => {}
                }

                // Focus-specific key handling
                match app.focus {
                    Focus::Input => match key.code {
                        KeyCode::Up => {
                            app.event_scroll = app.event_scroll.saturating_sub(1);
                            app.follow_mode = false;
                        }
                        KeyCode::Down => {
                            app.event_scroll = app.event_scroll.saturating_add(1);
                            app.follow_mode = false;
                        }
                        KeyCode::PageUp => {
                            app.event_scroll = app.event_scroll.saturating_sub(10);
                            app.follow_mode = false;
                        }
                        KeyCode::PageDown => {
                            app.event_scroll = app.event_scroll.saturating_add(10);
                            app.follow_mode = false;
                        }
                        KeyCode::Home => {
                            app.event_scroll = 0;
                            app.follow_mode = false;
                        }
                        KeyCode::End => {
                            app.event_scroll = usize::MAX;
                            app.follow_mode = true;
                        }
                        _ => {}
                    },
                    Focus::Sidebar => match key.code {
                        KeyCode::Left => {
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.sidebar_scroll = 0;
                            app.sidebar_selected = 0;
                        }
                        KeyCode::Right => {
                            app.sidebar_mode = SidebarMode::Models;
                            app.sidebar_scroll = 0;
                            app.sidebar_selected = 0;
                        }
                        KeyCode::Up => {
                            app.sidebar_scroll = app.sidebar_scroll.saturating_sub(1);
                            app.sidebar_selected = app.sidebar_selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            if app.sidebar_mode == SidebarMode::Models {
                                let model_count = models_generated::OPENROUTER_MODELS.len();
                                app.sidebar_selected =
                                    (app.sidebar_selected + 1).min(model_count.saturating_sub(1));
                                app.sidebar_scroll = (app.sidebar_scroll + 1)
                                    .min(model_count.saturating_sub(1) as u16);
                            }
                        }
                        KeyCode::Enter => {
                            if app.sidebar_mode == SidebarMode::Models {
                                let selected_model =
                                    models_generated::OPENROUTER_MODELS.get(app.sidebar_selected);
                                if let Some(model) = selected_model {
                                    let mut sm = app.sessionmanager.lock().await;
                                    if let Some(session) = sm.get_current_session_mut() {
                                        session.config.modelname = model.id.to_string();
                                        app.current_session_title =
                                            format_current_session_title(session);
                                    }
                                    app.eventlog.push(LogEntry::System(format!(
                                        "Switched model to: {}",
                                        model.id
                                    )));
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

async fn handle_session_command(app: &mut App, prompt: &str) -> bool {
    if prompt == "/exit" {
        app.inputstate = InputState::Exit;
        app.eventlog
            .push(LogEntry::System("Crust agent quitting.....".to_string()));
        return true;
    }

    if prompt == "/new" || prompt.starts_with("/new ") {
        let session_name = prompt.strip_prefix("/new").unwrap_or("").trim().to_string();
        if session_name.is_empty() {
            app.eventlog
                .push(LogEntry::Error("Usage: /new <session_name>".to_string()));
            return true;
        }

        let mut sessionmanager = app.sessionmanager.lock().await;
        if sessionmanager.session_name_exists(&session_name) {
            app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' already exists"
            )));
            return true;
        }
        let new_session = sessionmanager.create_session(session_name, app.system_message);
        app.current_session_title = format_current_session_title(new_session);
        app.eventlog.push(LogEntry::System(format!(
            "Created and switched to new session: {} ({})",
            new_session.name, new_session.id
        )));
        return true;
    }

    if prompt == "/list" {
        app.eventlog
            .push(LogEntry::System("--- Sessions ---".to_string()));
        let sessionmanager = app.sessionmanager.lock().await;
        for session in sessionmanager.list_sessions() {
            let current_marker = if sessionmanager
                .get_current_session()
                .is_some_and(|s| s.id == session.id)
            {
                " (current)"
            } else {
                ""
            };
            app.eventlog.push(LogEntry::System(format!(
                "ID: {} | Name: {} | Messages: {}{}",
                session.id,
                session.name,
                session.messages.len(),
                current_marker
            )));
        }
        app.eventlog
            .push(LogEntry::System("----------------".to_string()));
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/switch ") {
        let session_name = session_name.trim();
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => match sessionmanager.switch_session(&session_id) {
                Ok(session) => {
                    app.current_session_title = format_current_session_title(session);
                    app.eventlog.push(LogEntry::System(format!(
                        "Switched to session: {}",
                        session.name
                    )));
                }
                Err(e) => app.eventlog.push(LogEntry::Error(format!("Error: {e}"))),
            },
            None => app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' not found"
            ))),
        }
        return true;
    }

    false
}

fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(app.sidebar_width), Constraint::Min(10)])
        .split(area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(h_chunks[1]);

    let (sidebar_title, sidebar_content, sidebar_scroll) = {
        match app.sessionmanager.try_lock() {
            Ok(sm) => match app.sidebar_mode {
                SidebarMode::Sessions => {
                    let names = sm.list_session_names();
                    let content = if names.is_empty() {
                        "No sessions".to_string()
                    } else {
                        names.join("\n")
                    };
                    (" Sessions ", content, 0u16)
                }
                SidebarMode::Models => {
                    let current_id = sm
                        .get_current_session()
                        .map(|s| s.config.modelname.as_str())
                        .unwrap_or("");

                    let lines: Vec<String> = models_generated::OPENROUTER_MODELS
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let marker = if i == app.sidebar_selected {
                                "> "
                            } else {
                                "  "
                            };
                            let active = if m.id == current_id { " ●" } else { "" };
                            format!("{}{}{}", marker, m.id, active)
                        })
                        .collect();

                    let content = lines.join("\n");
                    (" Models ", content, app.sidebar_scroll)
                }
            },
            Err(_) => (" Loading... ", "Loading...".to_string(), 0u16),
        }
    };

    let border_style = if app.is_dragging_divider || app.hover_divider {
        Style::default().fg(Color::Yellow)
    } else if app.focus == Focus::Sidebar {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let sidebar = Paragraph::new(sidebar_content)
        .block(
            Block::default()
                .title(sidebar_title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .scroll((sidebar_scroll, 0))
        .wrap(Wrap { trim: false });

    let events_view_height = usize::from(v_chunks[0].height.saturating_sub(2));
    let events_view_width = usize::from(v_chunks[0].width.saturating_sub(2)).max(1);

    // Virtualize the event log: pre-wrap into rendered rows, then pass only the
    // visible window to ratatui. This avoids Paragraph::scroll's u16 limit.
    let rendered_lines: Vec<Line> = app
        .eventlog
        .iter()
        .flat_map(|entry| log_entry_to_wrapped_lines(entry, events_view_width))
        .collect();
    let rendered_height = rendered_lines.len();

    let max_scroll = rendered_height.saturating_sub(events_view_height);

    // Update scroll position
    if app.follow_mode {
        app.event_scroll = max_scroll;
    } else {
        app.event_scroll = app.event_scroll.min(max_scroll);
    }
    app.event_max_scroll = max_scroll;

    let visible_lines: Vec<Line> = rendered_lines
        .into_iter()
        .skip(app.event_scroll)
        .take(events_view_height)
        .collect();

    let events_pane = Paragraph::new(visible_lines).block(
        Block::default()
            .title(format!(" Agent Events - {:?} ", app.agentstate))
            .borders(Borders::ALL),
    );

    let cursor = if app.cursor_visible { "█" } else { " " };
    let input_text = format!("{}{}", app.inputbuffer, cursor);
    let input_pane = Paragraph::new(input_text)
        .block(
            Block::default()
                .title(app.current_session_title.as_str())
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::LightYellow))
        .wrap(Wrap { trim: false });

    frame.render_widget(sidebar, h_chunks[0]);
    frame.render_widget(events_pane, v_chunks[0]);
    frame.render_widget(input_pane, v_chunks[1]);
}

fn push_wrapped_log_lines(
    lines: &mut Vec<Line<'static>>,
    style: Style,
    text: String,
    width: usize,
) {
    let width = width.max(1);

    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(Line::styled(String::new(), style));
            continue;
        }

        let mut current = String::new();
        let mut current_width = 0usize;

        for ch in raw_line.chars() {
            // Most log output is ASCII. Treat tabs as wider so they don't
            // overflow badly; other non-ASCII chars may be approximate.
            let ch_width = if ch == '\t' { 4 } else { 1 };
            if current_width > 0 && current_width + ch_width > width {
                lines.push(Line::styled(std::mem::take(&mut current), style));
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }

        lines.push(Line::styled(current, style));
    }

    lines.push(Line::raw(""));
}

fn log_entry_to_wrapped_lines(entry: &LogEntry, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match entry {
        LogEntry::User(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Cyan),
            format!("User: {text}"),
            width,
        ),
        LogEntry::Assistant(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::White),
            format!(
                "{}Crust Agent:{}\n{}\n{}",
                "=".repeat(50),
                "=".repeat(50),
                text,
                "=".repeat(175),
            ),
            width,
        ),
        LogEntry::Thinking { kind, text } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::DarkGray),
            format!("Thinking block [{kind}]:\n{text}"),
            width,
        ),
        LogEntry::ToolCall { name, args } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Yellow),
            format!("tool call started: {name}\n{args}"),
            width,
        ),
        LogEntry::ToolResult { name, result } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Green),
            format!("tool call: {name} -> {result}"),
            width,
        ),
        LogEntry::System(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::LightBlue),
            text.clone(),
            width,
        ),
        LogEntry::Error(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Red),
            text.clone(),
            width,
        ),
    }
    lines
}

async fn emit_session_title(
    sessionmanager: &Arc<Mutex<SessionManager>>,
    event_tx: &mpsc::Sender<AgentEvent>,
) {
    let title = {
        let sm = sessionmanager.lock().await;
        sm.get_current_session().map(format_current_session_title)
    };
    if let Some(title) = title {
        let _ = event_tx
            .send(AgentEvent::SessionTitleUpdated { title })
            .await;
    }
}

async fn agent_main_run(
    sessionmanager: Arc<Mutex<SessionManager>>,
    prompt: String,
    event_tx: mpsc::Sender<AgentEvent>,
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
    emit_session_title(&sessionmanager, &event_tx).await;

    let config = {
        let sm = sessionmanager.lock().await;
        sm.get_current_config().unwrap()
    };

    for _ in 0..config.max_agent_steps {
        let current_messages = {
            let sm = sessionmanager.lock().await;
            sm.get_current_session().unwrap().messages.clone()
        };

        let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
            .model(&config.modelname)
            .messages(current_messages.clone())
            .typed_tools_batch(&[
                WebSearchParams::create_tool(),
                BashParams::create_tool(),
                ReadFileParams::create_tool(),
                EditFileParams::create_tool(),
                WriteFileParams::create_tool(),
            ])
            .temperature(0.2f64)
            .build()?;
        let client = config.client_builder()?;
        let response = match client.chat().create(&request).await {
            Ok(response) => response,
            Err(err) => {
                let modelname = config.modelname.clone();
                let _ = event_tx
                    .send(AgentEvent::Error {
                        message: format!(
                            "OpenRouter request failed for model `{modelname}`: {err:?}"
                        ),
                    })
                    .await;
                return Err(Box::new(err) as Box<dyn Error + Send + Sync>);
            }
        };

        if let Some(usage) = &response.usage {
            {
                let mut sm = sessionmanager.lock().await;
                sm.record_usage_to_current(
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                );
            }
            emit_session_title(&sessionmanager, &event_tx).await;
        }

        let Some(choice) = response.choices.first() else {
            let _ = event_tx
                .send(AgentEvent::Error {
                    message: "Crust Agent: No Response quitting.........".to_string(),
                })
                .await;
            break;
        };

        if let Some(details) = choice.reasoning_details() {
            for block in details {
                if let Some(text) = block.content() {
                    let _ = event_tx
                        .send(AgentEvent::Thinking {
                            kind: block.reasoning_type().to_string(),
                            text: text.to_string(),
                        })
                        .await;
                }
            }
        }

        if let Some(tool_calls) = choice.tool_calls() {
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_current(Message::assistant_with_tool_calls(
                    choice.content().unwrap_or(""),
                    tool_calls.to_vec(),
                ));
            }
            emit_session_title(&sessionmanager, &event_tx).await;

            for tool_call in tool_calls {
                let _ = event_tx
                    .send(AgentEvent::ToolCallStarted {
                        name: tool_call.name().to_string(),
                        args: tool_call.arguments_json().to_string(),
                    })
                    .await;

                let tool_result = execute_tool_call(tool_call).await?;

                let _ = event_tx
                    .send(AgentEvent::ToolCallFinished {
                        name: tool_call.name().to_string(),
                        result: tool_result.clone(),
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
                emit_session_title(&sessionmanager, &event_tx).await;
            }

            continue;
        } else {
            let final_content = choice.content().unwrap_or("").to_string();
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_current(Message::new(Role::Assistant, final_content.clone()));
            }
            emit_session_title(&sessionmanager, &event_tx).await;

            let _ = event_tx
                .send(AgentEvent::AssistantFinal {
                    text: final_content,
                })
                .await;
            return Ok(());
        }
    }

    let _ = event_tx.send(AgentEvent::MaxStepsReached).await;
    Ok(())
}

async fn execute_tool_call(tc: &ToolCall) -> Result<String, Box<dyn Error + Send + Sync>> {
    //Web Search tool exec

    if tc.is_tool::<WebSearchParams>() {
        let params = tc.parse_params::<WebSearchParams>()?;
        dotenvy::dotenv().ok();
        dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();
        let tavily_api_key = std::env::var("TAVILY_API_KEY")?;
        let tavily = Tavily::builder(tavily_api_key)
            .timeout(Duration::from_secs(45))
            .max_retries(5)
            .build()?;

        let response = tavily.search(params.query.clone()).await?;

        let mut title: Vec<String> = Vec::new();
        let mut content: Vec<String> = Vec::new();
        let mut url: Vec<String> = Vec::new();

        for (i, result) in response.results.into_iter().enumerate() {
            if i >= params.max_results {
                break;
            }

            title.push(result.title);
            content.push(result.content);
            url.push(result.url);
        }

        let answer = match response.answer {
            Some(answer) => answer,
            _ => "no answers".to_string(),
        };

        let follow_up_questions: Vec<String> = match response.follow_up_questions {
            Some(questions) => questions,
            _ => vec!["no follow ups".to_string()],
        };

        let websearch = json!({
            "answer":answer,
            "follow_up_questions":follow_up_questions,
            "results_titles":title,
            "results_urls":url,
            "results_contents":content,

        });

        return Ok(serde_json::to_string_pretty(&websearch)?);
    }
    // Bash Tool exec

    if tc.is_tool::<BashParams>() {
        let params = tc.parse_params::<BashParams>()?;
        let (exitcode, output, error) = rashf!("timeout {} {}", params.timeout, params.command)?;
        let bashresults = json!({
            "exitcode":exitcode,
            "output":output,
            "error":error,
        });
        return Ok(serde_json::to_string_pretty(&bashresults)?);
    }

    if tc.is_tool::<ReadFileParams>() {
        const DEFAULT_MAX_LINES: usize = 2_000;
        const DEFAULT_MAX_BYTES: usize = 50 * 1024;
        const MAX_LINES: usize = 20_000;

        let params = tc.parse_params::<ReadFileParams>()?;
        let filename = params.filename.clone();
        let start_line = params.offset.unwrap_or(1).max(1);
        let max_lines = params
            .limit
            .unwrap_or(DEFAULT_MAX_LINES)
            .clamp(1, MAX_LINES);

        let raw_content = std::fs::read_to_string(&filename)?;
        let all_lines: Vec<&str> = raw_content.lines().collect();
        let total_lines = all_lines.len();
        let file_size = raw_content.len();
        let start_index = start_line.saturating_sub(1);

        if start_index >= total_lines {
            let readfileresults = json!({
                "filename": filename,
                "offset": start_line,
                "limit": max_lines,
                "total_lines": total_lines,
                "file_size": file_size,
                "bytes_read": 0,
                "truncated": false,
                "next_offset": null,
                "content": "",
                "message": format!("Offset {start_line} is beyond end of file ({total_lines} lines total)"),
            });
            return Ok(serde_json::to_string_pretty(&readfileresults)?);
        }

        let mut selected_lines = Vec::new();
        let mut bytes_read = 0usize;
        let mut end_index = start_index;
        let user_limited = params.limit.is_some();

        for line in all_lines.iter().skip(start_index).take(max_lines) {
            let line_bytes = line.len() + 1; // Include newline separator.
            if !user_limited
                && !selected_lines.is_empty()
                && bytes_read + line_bytes > DEFAULT_MAX_BYTES
            {
                break;
            }
            selected_lines.push(*line);
            bytes_read += line_bytes;
            end_index += 1;
        }

        let truncated = end_index < total_lines;
        let next_offset = if truncated { Some(end_index + 1) } else { None };
        let mut content = selected_lines.join("\n");
        if truncated {
            let remaining = total_lines - end_index;
            content.push_str(&format!(
                "\n\n[{remaining} more lines in file. Use offset={} to continue.]",
                end_index + 1
            ));
        }

        let readfileresults = json!({
            "filename": filename,
            "offset": start_line,
            "limit": max_lines,
            "lines_read": selected_lines.len(),
            "total_lines": total_lines,
            "file_size": file_size,
            "bytes_read": bytes_read,
            "truncated": truncated,
            "next_offset": next_offset,
            "content": content,
        });

        return Ok(serde_json::to_string_pretty(&readfileresults)?);
    }

    if tc.is_tool::<WriteFileParams>() {
        let params = tc.parse_params::<WriteFileParams>()?;
        let filename = params.filename;
        let content = params.content;
        let file = std::fs::File::create(&filename)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes())?;

        let writerresults = json!({
            "filename":filename
        });
        return Ok(serde_json::to_string_pretty(&writerresults)?);
    }

    if tc.is_tool::<EditFileParams>() {
        let params = tc.parse_params::<EditFileParams>()?;

        let mut buf = std::fs::read_to_string(&params.filename)?;

        if let Some(offset) = buf.find(&params.oldcontent) {
            let end = offset + params.oldcontent.len();

            buf.replace_range(offset..end, &params.newcontent);

            std::fs::write(&params.filename, buf)?;

            let editfileresults = json!({
                "err" :"false",
                "content" : "edit file activated"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        } else {
            let editfileresults = json!({
                "err" :"true",
                "content" : "Oldcontent not found"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        }
    }

    Ok("unhandled tool:{tc.name()}".to_string())
}
