mod context;
mod models_generated;
mod session;

use context::ContextBuilder;
use futures_util::StreamExt;
use openrouter_rs::{
    Content,
    api::chat::Message,
    types::{Role, ToolCall, stream::StreamEvent, typed_tool::TypedTool},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use session::{Session, SessionManager};
use std::{
    env,
    error::Error,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tavily::Tavily;
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio::time;

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
    system_message: String,
    sessionmanager: Arc<Mutex<SessionManager>>,
    current_session_id: String,
    current_session_title: String,
    inputstate: InputState,
    agentstate: AgentState,
    inputbuffer: String,
    cursor_pos: usize,
    eventlog: Vec<LogEntry>,
    active_assistant_log_index: Option<usize>,
    active_thinking_log_index: Option<usize>,
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
    slash_command_selected: usize,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum AgentState {
    #[default]
    Idle,
    Done,
    Thinking,
    Tool,
    Error,
}

#[derive(Debug, Clone)]
pub struct TaggedAgentEvent {
    pub session_id: String,
    pub event: AgentEvent,
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
    SystemNotice { message: String },
    MaxStepsReached,
    SessionTitleUpdated { title: String },
    Finished,
}

#[derive(Debug, Clone)]
pub enum LogEntry {
    User(String),
    Assistant(String),      // In-progress streaming
    AssistantFinal(String), // Completed final response
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
    SlashCommands,
}

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/exit", "Exit the application"),
    ("/new <session_name>", "Create a new session"),
    ("/clear", "Clear current session history"),
    ("/context", "Show context status"),
    ("/compact", "Compact session history"),
    ("/delete <session_name>", "Delete a session"),
    ("/switch <session_name>", "Switch to a session"),
];

fn get_filtered_commands(filter: &str) -> Vec<(&'static str, &'static str)> {
    let filter_lower = filter.trim().to_lowercase();
    if filter_lower.is_empty() || filter_lower == "/" {
        SLASH_COMMANDS.to_vec()
    } else {
        SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&filter_lower))
            .collect()
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum Focus {
    #[default]
    Input,
    Sidebar,
}

fn append_delta(existing: &mut String, delta: &str) {
    if delta.is_empty() || existing.ends_with(delta) {
        return;
    }

    // Some providers/SDK layers send cumulative text instead of true deltas.
    // If so, append only the new suffix instead of duplicating text.
    if delta.starts_with(existing.as_str()) {
        existing.push_str(&delta[existing.len()..]);
        return;
    }

    // Also handle partial overlaps between adjacent chunks, e.g.
    // existing="hello wor" delta="world" -> append only "ld".
    let max_overlap = existing.len().min(delta.len());
    let mut delta_boundaries: Vec<usize> = delta.char_indices().map(|(idx, _)| idx).collect();
    delta_boundaries.push(delta.len());
    for overlap in delta_boundaries.into_iter().rev() {
        if overlap > 0 && overlap <= max_overlap && existing.ends_with(&delta[..overlap]) {
            existing.push_str(&delta[overlap..]);
            return;
        }
    }

    existing.push_str(delta);
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

fn context_pressure_message(
    context_builder: &ContextBuilder,
    config: &session::Config,
    prompt_tokens_sent: u32,
) -> Option<String> {
    if !context_builder.should_compact_from_api_usage(prompt_tokens_sent, config) {
        return None;
    }

    let context_window_tokens = context_builder.context_window_tokens(config);
    let ratio = context_builder.context_ratio_from_api_usage(prompt_tokens_sent, config) * 100.0;
    Some(format!(
        "Context pressure: sent {}/{} prompt toks ({ratio:.1}%). Run /compact before the next large turn.",
        format_token_count(prompt_tokens_sent as usize),
        format_token_count(context_window_tokens),
    ))
}

fn format_current_session_title(session: &Session) -> String {
    let context_builder = ContextBuilder::default();
    let context_window_tokens = context_builder.context_window_tokens(&session.config);
    let prompt_tokens = session.latest_prompt_tokens;
    let completion_tokens = session.latest_completion_tokens;
    let total_tokens = session.latest_total_tokens;
    let context_ratio =
        context_builder.context_ratio_from_api_usage(prompt_tokens, &session.config) * 100.0;

    format!(
        "{} - {} | {} | ctx {}/{} ({context_ratio:.1}%) | last p/c/t {}/{}/{} | total {} toks",
        session.name,
        session.id,
        session.config.modelname,
        format_token_count(prompt_tokens as usize),
        format_token_count(context_window_tokens),
        format_token_count(prompt_tokens as usize),
        format_token_count(completion_tokens as usize),
        format_token_count(total_tokens as usize),
        format_token_count(session.cumulative_total_tokens as usize)
    )
}

fn build_system_message() -> String {
    let shell = ShellKind::from_env().unwrap_or_else(|_| ShellKind::default_for_current_os());
    format!(
        "You are an AI agent given tools to help people. Use the shell tool to run {shell_name} commands in the workspace, the read/write/edit tools for files when you know filenames, and the web search tool to interface with the web.",
        shell_name = shell.display_name()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellKind {
    Bash,
    PowerShell,
    Cmd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellResult {
    exitcode: i32,
    output: String,
    error: String,
}

impl ShellKind {
    fn from_env() -> Result<Self, String> {
        let value = env::var("CRUST_SHELL").unwrap_or_else(|_| "auto".to_string());
        Self::from_config(&value, cfg!(windows))
    }

    fn from_config(value: &str, is_windows: bool) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::default_for_os(is_windows)),
            "bash" => Ok(Self::Bash),
            "powershell" | "pwsh" => Ok(Self::PowerShell),
            "cmd" => Ok(Self::Cmd),
            other => Err(format!(
                "Unsupported CRUST_SHELL value `{other}`. Use auto, bash, powershell, or cmd."
            )),
        }
    }

    fn default_for_current_os() -> Self {
        Self::default_for_os(cfg!(windows))
    }

    fn default_for_os(is_windows: bool) -> Self {
        if is_windows {
            Self::PowerShell
        } else {
            Self::Bash
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Bash => "Bash",
            Self::PowerShell => "PowerShell",
            Self::Cmd => "cmd.exe",
        }
    }

    fn command(self, command: &str) -> Command {
        let mut process = match self {
            Self::Bash => {
                let mut process = Command::new("bash");
                process.arg("-lc").arg(command);
                process
            }
            Self::PowerShell => {
                let mut process = Command::new("powershell.exe");
                process.arg("-NoProfile").arg("-Command").arg(command);
                process
            }
            Self::Cmd => {
                let mut process = Command::new("cmd.exe");
                process.arg("/C").arg(command);
                process
            }
        };
        process.kill_on_drop(true);
        process
    }
}

async fn run_shell_command(
    shell: ShellKind,
    command: &str,
    timeout_duration: Duration,
) -> Result<ShellResult, Box<dyn Error + Send + Sync>> {
    let mut process = shell.command(command);
    let output = match time::timeout(timeout_duration, process.output()).await {
        Ok(output) => output?,
        Err(_) => {
            let error = format!(
                "Command timed out after {} ms.",
                timeout_duration.as_millis()
            );
            return Ok(ShellResult {
                exitcode: -1,
                output: String::new(),
                error,
            });
        }
    };

    Ok(ShellResult {
        exitcode: output.status.code().unwrap_or(-1),
        output: String::from_utf8_lossy(&output.stdout).to_string(),
        error: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn normalize_tool_path(filename: &str) -> PathBuf {
    let converted = convert_windows_unix_style_path(filename);
    let path = Path::new(&converted);
    let mut normalized = PathBuf::new();

    for component in path.components() {
        if component == Component::CurDir {
            continue;
        }
        normalized.push(component.as_os_str());
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(filename)
    } else {
        normalized
    }
}

fn convert_windows_unix_style_path(filename: &str) -> String {
    if !cfg!(windows) {
        return filename.to_string();
    }

    let normalized_slashes = filename.replace('\\', "/");
    let bytes = normalized_slashes.as_bytes();
    if bytes.len() >= 7
        && normalized_slashes[..5].eq_ignore_ascii_case("/mnt/")
        && bytes[5].is_ascii_alphabetic()
        && bytes[6] == b'/'
    {
        let drive = char::from(bytes[5]).to_ascii_uppercase();
        return format!("{drive}:{}", &normalized_slashes[6..]);
    }

    if bytes.len() >= 4 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        let drive = char::from(bytes[1]).to_ascii_uppercase();
        return format!("{drive}:{}", &normalized_slashes[2..]);
    }

    filename.to_string()
}

impl App {
    pub fn new() -> Self {
        // Initialize messages with system prompt
        let system_message = build_system_message();

        // Initialize Session Manager
        let mut session_manager = SessionManager::new();

        let mut eventlog = Vec::new();
        let current_session_title;
        let current_session_id;
        let initial_scroll;
        if session_manager.load_most_recent_session() {
            let current_session = session_manager.get_current_session().unwrap();
            current_session_title = format_current_session_title(current_session);
            current_session_id = current_session.id.clone();

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
                session_manager.create_session("Default".to_string(), &system_message);
            current_session_title = format_current_session_title(default_session);
            current_session_id = default_session.id.clone();
            eventlog.push(LogEntry::System(format!(
                "Created session: {}",
                default_session.name
            )));
            initial_scroll = 0;
        }

        Self {
            system_message,
            sessionmanager: Arc::new(Mutex::new(session_manager)),
            current_session_id,
            current_session_title,
            inputstate: InputState::default(),
            agentstate: AgentState::default(),
            inputbuffer: "".to_string(),
            cursor_pos: 0,
            eventlog,
            active_assistant_log_index: None,
            active_thinking_log_index: None,
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
            slash_command_selected: 0,
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
                    // Replace the streaming entry with a final entry for better formatting
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
            AgentEvent::Finished => {
                self.active_assistant_log_index = None;
                self.active_thinking_log_index = None;
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

// tool setup for shell commands
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
struct ShellParams {
    timeout: u32,
    command: String,
}

impl TypedTool for ShellParams {
    fn name() -> &'static str {
        "shell_calling_tool"
    }

    fn description() -> &'static str {
        "Run a command in the configured workspace shell"
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
    session::load_env();

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
    let (agent_tx, mut agent_rx) = mpsc::channel::<TaggedAgentEvent>(100);

    loop {
        while let Ok(tagged) = agent_rx.try_recv() {
            // Filter out stale events from a previous session
            if tagged.session_id == app.current_session_id {
                app.handle_agent_event(tagged.event);
            }
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
                        app.inputbuffer.insert(app.cursor_pos, c);
                        app.cursor_pos += 1;
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        // Auto-switch sidebar to slash-command mode when input starts with '/'
                        if app.inputbuffer.starts_with('/') {
                            app.sidebar_mode = SidebarMode::SlashCommands;
                            app.slash_command_selected = 0;
                            app.sidebar_scroll = 0;
                        } else if app.sidebar_mode == SidebarMode::SlashCommands {
                            // User typed something else after '/', exit slash-command mode
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.slash_command_selected = 0;
                        }
                        continue;
                    }
                    KeyCode::Backspace if app.focus == Focus::Input => {
                        if app.cursor_pos > 0 {
                            app.cursor_pos -= 1;
                            app.inputbuffer.remove(app.cursor_pos);
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        // If buffer is now empty or doesn't start with '/', exit slash-command mode
                        if app.inputbuffer.is_empty() || !app.inputbuffer.starts_with('/') {
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.slash_command_selected = 0;
                        } else if app.sidebar_mode == SidebarMode::SlashCommands {
                            // Reset selection to top when filter changes
                            app.slash_command_selected = 0;
                            app.sidebar_scroll = 0;
                        }
                        continue;
                    }
                    KeyCode::Left if app.focus == Focus::Input => {
                        if app.cursor_pos > 0 {
                            app.cursor_pos -= 1;
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Right if app.focus == Focus::Input => {
                        if app.cursor_pos < app.inputbuffer.len() {
                            app.cursor_pos += 1;
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Enter if app.focus == Focus::Input => {
                        // If in slash-command mode with a selection, use that command
                        let prompt = if app.sidebar_mode == SidebarMode::SlashCommands {
                            let filtered = get_filtered_commands(&app.inputbuffer);
                            if let Some((cmd, _)) = filtered.get(app.slash_command_selected) {
                                app.inputbuffer.clear();
                                cmd.to_string()
                            } else {
                                app.inputbuffer.trim().to_string()
                            }
                        } else {
                            app.inputbuffer.trim().to_string()
                        };
                        app.inputbuffer.clear();
                        app.cursor_pos = 0;
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        // Exit slash-command mode after submit
                        app.sidebar_mode = SidebarMode::Sessions;
                        app.slash_command_selected = 0;

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

                        // Capture current session ID before spawning task
                        let current_session_id = app.current_session_id.clone();

                        app.handle_agent_event(AgentEvent::UserSubmitted {
                            prompt: prompt.clone(),
                        });
                        terminal.draw(|f| ui(f, app))?;

                        let sessionmanager = Arc::clone(&app.sessionmanager);
                        let tx = agent_tx.clone();
                        let spawn_session_id = current_session_id.clone();
                        app.agent_task = Some(tokio::spawn(async move {
                            if let Err(err) = agent_main_run(
                                sessionmanager,
                                prompt,
                                current_session_id,
                                tx.clone(),
                            )
                            .await
                            {
                                let _ = tx
                                    .send(TaggedAgentEvent {
                                        session_id: spawn_session_id.clone(),
                                        event: AgentEvent::Error {
                                            message: err.to_string(),
                                        },
                                    })
                                    .await;
                            }
                            let _ = tx
                                .send(TaggedAgentEvent {
                                    session_id: spawn_session_id,
                                    event: AgentEvent::Finished,
                                })
                                .await;
                        }));
                        continue;
                    }
                    _ => {}
                }

                // Focus-specific key handling
                match app.focus {
                    Focus::Input => match key.code {
                        KeyCode::Up => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected =
                                    app.slash_command_selected.saturating_sub(1);
                                app.sidebar_scroll = app.sidebar_scroll.saturating_sub(1);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_sub(1);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::Down => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                let max = filtered.len().saturating_sub(1);
                                app.slash_command_selected =
                                    (app.slash_command_selected + 1).min(max);
                                app.sidebar_scroll = (app.sidebar_scroll + 1).min(max as u16);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_add(1);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::PageUp => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected =
                                    app.slash_command_selected.saturating_sub(5);
                                app.sidebar_scroll = app.sidebar_scroll.saturating_sub(5);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_sub(10);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::PageDown => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                let max = filtered.len().saturating_sub(1);
                                app.slash_command_selected =
                                    (app.slash_command_selected + 5).min(max);
                                app.sidebar_scroll = (app.sidebar_scroll + 5).min(max as u16);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_add(10);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::Home => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected = 0;
                                app.sidebar_scroll = 0;
                            } else {
                                app.event_scroll = 0;
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::End => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                app.slash_command_selected = filtered.len().saturating_sub(1);
                                app.sidebar_scroll =
                                    filtered.len().saturating_sub(1).min(u16::MAX as usize) as u16;
                            } else {
                                app.event_scroll = usize::MAX;
                                app.follow_mode = true;
                            }
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
        let new_session = sessionmanager.create_session(session_name, &app.system_message);
        app.current_session_title = format_current_session_title(new_session);
        app.eventlog.push(LogEntry::System(format!(
            "Created and switched to new session: {} ({})",
            new_session.name, new_session.id
        )));
        return true;
    }

    if prompt == "/clear" {
        let mut sessionmanager = app.sessionmanager.lock().await;
        sessionmanager.clear_current_session();
        if let Some(session) = sessionmanager.get_current_session() {
            app.current_session_title = format_current_session_title(session);
        }
        app.eventlog.clear();
        app.eventlog.push(LogEntry::System(
            "Cleared current session history.".to_string(),
        ));
        app.event_scroll = 0;
        app.follow_mode = true;
        return true;
    }

    if prompt == "/context" {
        let context_builder = ContextBuilder::default();
        let sessionmanager = app.sessionmanager.lock().await;
        if let Some(session) = sessionmanager.get_current_session() {
            let context = context_builder.build_context(session, &session.config);
            let estimated_tokens = context_builder.estimate_context_tokens(&context);
            let context_window = context_builder.context_window_tokens(&session.config);
            let estimated_ratio =
                context_builder.estimated_context_ratio(&context, &session.config);
            let last_api_ratio = context_builder
                .context_ratio_from_api_usage(session.latest_prompt_tokens, &session.config);
            app.eventlog.push(LogEntry::System(format!(
                "Context status:\nmodel: {}\ncontext window: {} toks\nbuilt context: {} messages, est. {} toks ({:.1}%)\nlast OpenRouter prompt usage: {} toks ({:.1}%)\nsummary: {}\ncompacted_until: {} / {} messages",
                session.config.modelname,
                format_token_count(context_window),
                context.len(),
                format_token_count(estimated_tokens),
                estimated_ratio * 100.0,
                format_token_count(session.latest_prompt_tokens as usize),
                last_api_ratio * 100.0,
                if session.summary.is_some() { "present" } else { "none" },
                session.compacted_until,
                session.messages.len(),
            )));
        }
        return true;
    }

    if prompt == "/compact" {
        let context_builder = ContextBuilder::default();
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.compact_current_session_to_recent(context_builder.min_recent_messages)
        {
            Ok(cutoff) => {
                if let Some(session) = sessionmanager.get_current_session() {
                    app.current_session_title = format_current_session_title(session);
                }
                app.eventlog.push(LogEntry::System(format!(
                    "Compacted session history through message index {cutoff}. The model context will now use the summary plus the recent tail."
                )));
            }
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Compact failed: {err}"))),
        }
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/delete ") {
        let session_name = session_name.trim();
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => {
                if let Some(current) = sessionmanager.get_current_session() {
                    if current.name == session_name {
                        app.eventlog.push(LogEntry::Error(format!(
                            "Cannot delete the active session '{session_name}'"
                        )));
                        return true;
                    }
                }
                match sessionmanager.delete_session(&session_id) {
                    Ok(_) => app
                        .eventlog
                        .push(LogEntry::System(format!("Deleted session: {session_name}"))),
                    Err(e) => app.eventlog.push(LogEntry::Error(format!("Error: {e}"))),
                }
            }
            None => app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' not found"
            ))),
        }
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/switch ") {
        let session_name = session_name.trim();
        // Block session switching while agent is running
        if app.agent_task.is_some() {
            app.eventlog.push(LogEntry::Error(
                "Cannot switch sessions while agent is running. Press Esc to cancel first."
                    .to_string(),
            ));
            return true;
        }
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => match sessionmanager.switch_session(&session_id) {
                Ok(session) => {
                    app.current_session_id = session.id.clone();
                    app.current_session_title = format_current_session_title(session);
                    // Clear and rebuild event log from the switched session's messages
                    app.eventlog.clear();
                    app.active_assistant_log_index = None;
                    app.active_thinking_log_index = None;
                    for message in &session.messages {
                        if let Some(log_entry) = message_to_log_entry(message) {
                            app.eventlog.push(log_entry);
                        }
                    }
                    app.eventlog.push(LogEntry::System(format!(
                        "Switched to session: {} ({} messages)",
                        session.name,
                        session.messages.len()
                    )));
                    app.event_scroll = 0;
                    app.follow_mode = true;
                    app.agentstate = AgentState::Idle;
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
        match app.sidebar_mode {
            SidebarMode::SlashCommands => {
                let filtered = get_filtered_commands(&app.inputbuffer);
                let lines: Vec<String> = filtered
                    .iter()
                    .enumerate()
                    .map(|(i, (cmd, _desc))| {
                        let marker = if i == app.slash_command_selected {
                            "> "
                        } else {
                            "  "
                        };
                        format!("{}{}", marker, cmd)
                    })
                    .collect();
                let content = if lines.is_empty() {
                    "No matching commands".to_string()
                } else {
                    lines.join("\n")
                };
                (" Commands ", content, app.sidebar_scroll)
            }
            _ => match app.sessionmanager.try_lock() {
                Ok(sm) => match app.sidebar_mode {
                    SidebarMode::Sessions => {
                        let current_name = sm
                            .get_current_session()
                            .map(|s| s.name.as_str())
                            .unwrap_or("");
                        let names = sm.list_session_names();
                        let lines: Vec<String> = names
                            .iter()
                            .map(|name| {
                                let active = if *name == current_name { " ●" } else { "" };
                                format!("{}{}", name, active)
                            })
                            .collect();
                        let content = if lines.is_empty() {
                            "No sessions".to_string()
                        } else {
                            lines.join("\n")
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
                    // SlashCommands already handled above
                    SidebarMode::SlashCommands => unreachable!(),
                },
                Err(_) => (" Loading... ", "Loading...".to_string(), 0u16),
            },
        }
    };

    let border_style = if app.is_dragging_divider || app.hover_divider {
        Style::default().fg(Color::Yellow)
    } else if app.focus == Focus::Sidebar || app.sidebar_mode == SidebarMode::SlashCommands {
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

    let event_border_color = match app.agentstate {
        AgentState::Thinking | AgentState::Tool => Color::Yellow,
        AgentState::Done => Color::Green,
        AgentState::Error => Color::Red,
        AgentState::Idle => Color::Blue,
    };

    let events_pane = Paragraph::new(visible_lines).block(
        Block::default()
            .title(format!(" Agent Events - {:?} ", app.agentstate))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(event_border_color)),
    );

    let input_border_color = if app.focus == Focus::Input {
        Color::Rgb(255, 165, 0) // Orange
    } else {
        Color::White
    };

    let cursor = if app.cursor_visible { "█" } else { " " };
    let input_text = format!(
        "{}{}{}",
        &app.inputbuffer[..app.cursor_pos.min(app.inputbuffer.len())],
        cursor,
        &app.inputbuffer[app.cursor_pos.min(app.inputbuffer.len())..]
    );
    let input_pane = Paragraph::new(input_text)
        .block(
            Block::default()
                .title(app.current_session_title.as_str())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(input_border_color)),
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
            format!("{text}"),
            width,
        ),
        LogEntry::AssistantFinal(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::White),
            format!(
                "{} FINAL RESPONSE {}\n{}\n{}",
                "▶".repeat(20),
                "◀".repeat(20),
                text,
                "─".repeat(width.min(80)),
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

async fn agent_main_run(
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

    let config = {
        let sm = sessionmanager.lock().await;
        sm.get_current_config().unwrap()
    };

    let client = config.client_builder()?;
    let context_builder = ContextBuilder::default();

    for _ in 0..config.max_agent_steps {
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

        let request = openrouter_rs::api::chat::ChatCompletionRequest::builder()
            .model(&config.modelname)
            .messages(current_messages)
            .session_id(session_id.clone())
            .typed_tools_batch(&[
                WebSearchParams::create_tool(),
                ShellParams::create_tool(),
                ReadFileParams::create_tool(),
                EditFileParams::create_tool(),
                WriteFileParams::create_tool(),
            ])
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

        while let Some(event) = stream.next().await {
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
                    // ToolAwareStream can expose the same provider reasoning both as a
                    // plain ReasoningDelta and as structured ReasoningDetailsDelta.
                    // Showing both creates the repeated tiny blocks in the event pane.
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
                        // Add error as tool response so conversation can continue
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

async fn execute_tool_call(tc: &ToolCall) -> Result<String, Box<dyn Error + Send + Sync>> {
    //Web Search tool exec

    if tc.is_tool::<WebSearchParams>() {
        let params = tc.parse_params::<WebSearchParams>()?;
        session::load_env();
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
    // Shell Tool exec

    if tc.is_tool::<ShellParams>() {
        let params = tc.parse_params::<ShellParams>()?;
        let shell = ShellKind::from_env()?;
        let result = run_shell_command(
            shell,
            &params.command,
            Duration::from_secs(params.timeout.into()),
        )
        .await?;
        let shellresults = json!({
            "exitcode":result.exitcode,
            "output":result.output,
            "error":result.error,
        });
        return Ok(serde_json::to_string_pretty(&shellresults)?);
    }

    if tc.is_tool::<ReadFileParams>() {
        const DEFAULT_MAX_LINES: usize = 2_000;
        const DEFAULT_MAX_BYTES: usize = 50 * 1024;
        const MAX_LINES: usize = 20_000;

        let params = tc.parse_params::<ReadFileParams>()?;
        let filename = params.filename.clone();
        let filepath = normalize_tool_path(&filename);
        let start_line = params.offset.unwrap_or(1).max(1);
        let max_lines = params
            .limit
            .unwrap_or(DEFAULT_MAX_LINES)
            .clamp(1, MAX_LINES);

        let raw_content = std::fs::read_to_string(&filepath)?;
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
        let filepath = normalize_tool_path(&filename);
        let file = std::fs::File::create(&filepath)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes())?;

        let writerresults = json!({
            "filename":filename
        });
        return Ok(serde_json::to_string_pretty(&writerresults)?);
    }

    if tc.is_tool::<EditFileParams>() {
        let params = tc.parse_params::<EditFileParams>()?;
        let filepath = normalize_tool_path(&params.filename);

        let mut buf = std::fs::read_to_string(&filepath)?;

        if let Some(offset) = buf.find(&params.oldcontent) {
            let end = offset + params.oldcontent.len();

            buf.replace_range(offset..end, &params.newcontent);

            std::fs::write(&filepath, buf)?;

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
