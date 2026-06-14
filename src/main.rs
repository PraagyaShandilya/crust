mod session;

use openrouter_rs::{
    Content,
    api::chat::Message,
    types::{Role, ToolCall, typed_tool::TypedTool},
};
use rsbash::rashf;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use session::SessionManager;
use std::{
    error::Error,
    io::{self, Read, Seek, SeekFrom, Write},
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
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
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
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum AgentState {
    #[default]
    Running,
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
    AssistantFinal { text: String },
    Error { message: String },
    MaxStepsReached,
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
        if session_manager.load_most_recent_session() {
            let current_session = session_manager.get_current_session().unwrap();
            current_session_title = format!("{} - {}", current_session.name, current_session.id);
            eventlog.push(LogEntry::System(format!(
                "Loaded session: {}",
                current_session.name
            )));
        } else {
            let default_session =
                session_manager.create_session("Default".to_string(), system_message);
            current_session_title = format!("{} - {}", default_session.name, default_session.id);
            eventlog.push(LogEntry::System(format!(
                "Created session: {}",
                default_session.name
            )));
        }

        Self {
            system_message,
            sessionmanager: Arc::new(Mutex::new(session_manager)),
            current_session_title,
            inputstate: InputState::default(),
            agentstate: AgentState::default(),
            inputbuffer: "".to_string(),
            eventlog,
            event_scroll: 0,
            event_max_scroll: 0,
            cursor_visible: true,
            last_cursor_toggle: Instant::now(),
            agent_task: None,
            sidebar_width: 30,
            is_dragging_divider: false,
            hover_divider: false,
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
            AgentEvent::AssistantFinal { text } => {
                self.eventlog.push(LogEntry::Assistant(text));
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
            AgentEvent::Finished => {
                self.agent_task = None;
                self.agentstate = AgentState::Done;
            }
        }
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
    offset: u64,
    limit: usize,
}

impl TypedTool for ReadFileParams {
    fn name() -> &'static str {
        "read_file_tool"
    }
    fn description() -> &'static str {
        "Read a file using its file name, starting from the offest value and ending at the limit value, and get a string containing its contents, increment the offset with a fixed limit to read an entire file in chunks"
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

                app.hover_divider =
                    mouse_event.column.abs_diff(divider_x) <= 1 && mouse_event.row < area.height;

                match mouse_event.kind {
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

                match key.code {
                    KeyCode::Char(c) => {
                        app.inputbuffer.push(c);
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                    }
                    KeyCode::Backspace => {
                        app.inputbuffer.pop();
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                    }
                    KeyCode::Up => {
                        app.event_scroll = app.event_scroll.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        app.event_scroll = app.event_scroll.saturating_add(1).min(app.event_max_scroll);
                    }
                    KeyCode::PageUp => {
                        app.event_scroll = app.event_scroll.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        app.event_scroll = app
                            .event_scroll
                            .saturating_add(10)
                            .min(app.event_max_scroll);
                    }
                    KeyCode::Home => {
                        app.event_scroll = 0;
                    }
                    KeyCode::End => {
                        app.event_scroll = app.event_max_scroll;
                    }
                    KeyCode::Esc => {
                        if let Some(task) = app.agent_task.take() {
                            task.abort();
                            app.handle_agent_event(AgentEvent::Error {
                                message: "Agent run cancelled by user.".to_string(),
                            });
                        } else {
                            break;
                        }
                    }
                    KeyCode::Enter => {
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
                            if let Err(err) = agent_main_run(sessionmanager, prompt, tx.clone()).await {
                                let _ = tx
                                    .send(AgentEvent::Error {
                                        message: err.to_string(),
                                    })
                                    .await;
                            }
                            let _ = tx.send(AgentEvent::Finished).await;
                        }));
                    }
                    _ => {}
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
        app.current_session_title = format!("{} - {}", new_session.name, new_session.id);
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
                    app.current_session_title = format!("{} - {}", session.name, session.id);
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
        .constraints([
            Constraint::Length(app.sidebar_width),
            Constraint::Min(10),
        ])
        .split(area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(h_chunks[1]);

    let sidebar_content = match app.sessionmanager.try_lock() {
        Ok(sm) => {
            let names = sm.list_session_names();
            if names.is_empty() {
                "No sessions".to_string()
            } else {
                names.join("\n")
            }
        }
        Err(_) => "Loading...".to_string(),
    };

    let border_style = if app.is_dragging_divider || app.hover_divider {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let sidebar = Paragraph::new(sidebar_content)
        .block(
            Block::default()
                .title(" Sessions ")
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false });

    let lines: Vec<Line> = app.eventlog.iter().flat_map(log_entry_to_lines).collect();

    let events_view_height = usize::from(v_chunks[0].height.saturating_sub(2));
    let events_view_width = usize::from(v_chunks[0].width.saturating_sub(2)).max(1);
    let rendered_height: usize = lines
        .iter()
        .map(|line| line.width().div_ceil(events_view_width).max(1))
        .sum();
    let max_scroll = rendered_height.saturating_sub(events_view_height);
    let scroll_growth = max_scroll.saturating_sub(app.event_max_scroll);
    app.event_scroll = app
        .event_scroll
        .saturating_add(scroll_growth)
        .min(max_scroll);
    app.event_max_scroll = max_scroll;
    let event_scroll = app.event_scroll.min(usize::from(u16::MAX)) as u16;

    let events_pane = Paragraph::new(lines)
        .block(
            Block::default()
                .title(format!(" Agent Events - {:?} ", app.agentstate))
                .borders(Borders::ALL),
        )
        .scroll((event_scroll, 0))
        .wrap(Wrap { trim: false });

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

fn push_log_lines(lines: &mut Vec<Line<'static>>, style: Style, text: String) {
    for line in text.lines() {
        lines.push(Line::styled(line.to_string(), style));
    }
    lines.push(Line::raw(""));
}

fn log_entry_to_lines(entry: &LogEntry) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match entry {
        LogEntry::User(text) => push_log_lines(
            &mut lines,
            Style::default().fg(Color::Cyan),
            format!("User: {text}"),
        ),
        LogEntry::Assistant(text) => push_log_lines(
            &mut lines,
            Style::default().fg(Color::White),
            format!(
                "{}Crust Agent:{}\n{}\n{}",
                "=".repeat(50),
                "=".repeat(50),
                text,
                "=".repeat(175),
            ),
        ),
        LogEntry::Thinking { kind, text } => push_log_lines(
            &mut lines,
            Style::default().fg(Color::DarkGray),
            format!("Thinking block [{kind}]:\n{text}"),
        ),
        LogEntry::ToolCall { name, args } => push_log_lines(
            &mut lines,
            Style::default().fg(Color::Yellow),
            format!("tool call started: {name}\n{args}"),
        ),
        LogEntry::ToolResult { name, result } => push_log_lines(
            &mut lines,
            Style::default().fg(Color::Green),
            format!("tool call: {name} -> {result}"),
        ),
        LogEntry::System(text) => push_log_lines(
            &mut lines,
            Style::default().fg(Color::LightBlue),
            text.clone(),
        ),
        LogEntry::Error(text) => {
            push_log_lines(&mut lines, Style::default().fg(Color::Red), text.clone())
        }
    }
    lines
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
            }

            continue;
        } else {
            let final_content = choice.content().unwrap_or("").to_string();
            {
                let mut sm = sessionmanager.lock().await;
                sm.add_message_to_current(Message::new(Role::Assistant, final_content.clone()));
            }

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
        let params = tc.parse_params::<ReadFileParams>()?;
        let cursor_start = params.offset;
        let filename = params.filename.clone();
        let limit = params.limit.min(1_048_576);
        let file = std::fs::File::open(&filename)?;
        let mut reader = std::io::BufReader::new(file);
        reader.seek(SeekFrom::Start(params.offset))?;

        let mut buffer = vec![0; limit];
        let bytes_read = reader.read(&mut buffer)?;
        buffer.truncate(bytes_read);
        let content = String::from_utf8_lossy(&buffer).to_string();

        let readfileresults = json!(
            {
                "filename" : filename,
                "offset" : cursor_start,
                "bytes_read" : bytes_read,
                "content": content,
            }
        );

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
