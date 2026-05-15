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
    time::{Duration, Instant},
};
use tavily::Tavily;

use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

fn generate_landing_page() -> String {
    r#"
        ▒▒          ▒▒                                ▒▒          ▒▒
        ▒▒          ▒▒                                ▒▒          ▒▒
      ▒▒▒▒▒▒      ▒▒▒▒▒▒                            ▒▒▒▒▒▒      ▒▒▒▒▒▒
      ▒▒▒▒▒▒▒▒  ▒▒▒▒▒▒▒▒                            ▒▒▒▒▒▒▒▒  ▒▒▒▒▒▒▒▒
      ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒                            ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ████        ████        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒
        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ████        ████        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒
          ▒▒▒▒▒▒▒▒▒▒                                    ▒▒▒▒▒▒▒▒▒▒
            ▒▒▒▒▒▒            ▒▒▒▒        ▒▒▒▒            ▒▒▒▒▒▒
            ▒▒▒▒▒▒            ▒▒▒▒        ▒▒▒▒            ▒▒▒▒▒▒
            ▒▒▒▒▒▒        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ▒▒▒▒▒▒
            ▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒▒▒▒▒
              ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
            ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒
          ▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒
        ▒▒▒▒        ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒        ▒▒▒▒
        ▒▒▒▒        ▒▒▒▒    ▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒    ▒▒▒▒        ▒▒▒▒
        ▒▒▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒▒▒
          ▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒
          ▒▒      ▒▒▒▒                                ▒▒▒▒      ▒▒
          ░░        ▒▒                                ▒▒░░
                    ▒▒                                ▒▒
                    WELCOME TO THE CRUST CODING ASSISTANT

    "#
    .to_string()
}

// setup for tui
#[derive(Debug)]
pub struct App {
    system_message: &'static str,
    sessionmanager: SessionManager,
    inputstate: InputState,
    agentstate: AgentState,
    inputbuffer: String,
    eventlog: Vec<String>,
    cursor_visible: bool,
    last_cursor_toggle: Instant,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum AgentState {
    #[default]
    Running,
    Done,
    Thinking,
    Tool,
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
        if session_manager.load_most_recent_session() {
            let current_session = session_manager.get_current_session().unwrap();
            eventlog.push(format!("Loaded session: {}", current_session.name));
        } else {
            let default_session =
                session_manager.create_session("Default".to_string(), system_message);
            eventlog.push(format!("Created session: {}", default_session.name));
        }

        Self {
            system_message,
            sessionmanager: session_manager,
            inputstate: InputState::default(),
            agentstate: AgentState::default(),
            inputbuffer: "".to_string(),
            eventlog,
            cursor_visible: true,
            last_cursor_toggle: Instant::now(),
        }
    }

    pub fn get_sessionmanager(&self) -> SessionManager {
        self.sessionmanager.clone()
    }

    pub fn get_inputstate(&self) -> InputState {
        self.inputstate.clone()
    }

    pub fn get_agentstate(&self) -> AgentState {
        self.agentstate.clone()
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
        "Read a file using its file name, starting from the offest value and ending at the limit value, and get a string containing its contents"
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
    loop {
        if app.last_cursor_toggle.elapsed() >= Duration::from_millis(500) {
            app.cursor_visible = !app.cursor_visible;
            app.last_cursor_toggle = Instant::now();
        }

        terminal.draw(|f| ui(f, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

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
            KeyCode::Esc => break,
            KeyCode::Enter => {
                let prompt = app.inputbuffer.trim().to_string();
                app.inputbuffer.clear();
                app.cursor_visible = true;
                app.last_cursor_toggle = Instant::now();

                if prompt.is_empty() {
                    continue;
                }
                if handle_session_command(app, &prompt) {
                    if app.inputstate == InputState::Exit {
                        break;
                    }
                    continue;
                }

                app.agentstate = AgentState::Thinking;
                terminal.draw(|f| ui(f, app))?;

                match agent_main_run(app, prompt).await {
                    Ok(response) => {
                        app.agentstate = AgentState::Done;
                        app.eventlog.push(response);
                    }
                    Err(err) => {
                        app.agentstate = AgentState::Done;
                        app.eventlog.push(format!("Agent error: {err}"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

fn handle_session_command(app: &mut App, prompt: &str) -> bool {
    if prompt == "/exit" {
        app.inputstate = InputState::Exit;
        app.eventlog.push("Crust agent quitting.....".to_string());
        return true;
    }

    if prompt == "/new" || prompt.starts_with("/new ") {
        let session_name = prompt.strip_prefix("/new").unwrap_or("").trim().to_string();
        if session_name.is_empty() {
            app.eventlog.push("Usage: /new <session_name>".to_string());
            return true;
        }
        if app.sessionmanager.session_name_exists(&session_name) {
            app.eventlog.push(format!(
                "Error: session named '{session_name}' already exists"
            ));
            return true;
        }
        let new_session = app
            .sessionmanager
            .create_session(session_name, app.system_message);
        app.eventlog.push(format!(
            "Created and switched to new session: {} ({})",
            new_session.name, new_session.id
        ));
        return true;
    }

    if prompt == "/list" {
        app.eventlog.push("--- Sessions ---".to_string());
        for session in app.sessionmanager.list_sessions() {
            let current_marker = if app
                .sessionmanager
                .get_current_session()
                .is_some_and(|s| s.id == session.id)
            {
                " (current)"
            } else {
                ""
            };
            app.eventlog.push(format!(
                "ID: {} | Name: {} | Messages: {}{}",
                session.id,
                session.name,
                session.messages.len(),
                current_marker
            ));
        }
        app.eventlog.push("----------------".to_string());
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/switch ") {
        let session_name = session_name.trim();
        match app.sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => match app.sessionmanager.switch_session(&session_id) {
                Ok(session) => app
                    .eventlog
                    .push(format!("Switched to session: {}", session.name)),
                Err(e) => app.eventlog.push(format!("Error: {e}")),
            },
            None => app
                .eventlog
                .push(format!("Error: session named '{session_name}' not found")),
        }
        return true;
    }

    false
}

fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(frame.area());

    let mut lines: Vec<Line> = app
        .eventlog
        .iter()
        .flat_map(|event| [Line::raw(event.clone()), Line::raw("")])
        .collect();

    if let Some(session) = app.sessionmanager.get_current_session() {
        lines.push(Line::styled(
            format!("Current session: {} ({})", session.name, session.id),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let events_pane = Paragraph::new(lines)
        .block(
            Block::default()
                .title(format!(" Agent Events - {:?} ", app.agentstate))
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });

    let cursor = if app.cursor_visible { "█" } else { " " };
    let input_text = format!("{}{}", app.inputbuffer, cursor);
    let input_pane = Paragraph::new(input_text)
        .block(Block::default().title(" Input ").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });

    frame.render_widget(events_pane, chunks[0]);
    frame.render_widget(input_pane, chunks[1]);
}

async fn agent_main_run(app: &mut App, prompt: String) -> Result<String, Box<dyn Error>> {
    // Add user message to session
    let user_message = Message {
        role: Role::User,
        content: Content::Text(prompt),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    };
    app.sessionmanager.add_message_to_current(user_message);

    let config = app.sessionmanager.get_current_config().unwrap();

    for _ in 0..config.max_agent_steps {
        // Get current messages for API request
        let current_messages = app
            .sessionmanager
            .get_current_session()
            .unwrap()
            .messages
            .clone();

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
                let modelname = config.modelname;
                app.eventlog
                    .push(format!("OpenRouter request failed for model `{modelname}`: {err:?}"));
                return Err(Box::new(err) as Box<dyn Error>);
            }
        };

        let Some(choice) = response.choices.first() else {
            app.eventlog
                .push("Crust Agent: No Response quitting.........".to_string());
            break;
        };

        if let Some(details) = choice.reasoning_details() {
            for block in details {
                if let Some(text) = block.content() {
                    app.eventlog
                        .push(format!("Thinking block [{}]:\n{}", block.reasoning_type(), text));
                }
            }
        }

        if let Some(tool_calls) = choice.tool_calls() {
            // Add assistant message with tool calls to session
            app.sessionmanager.add_message_to_current(Message::assistant_with_tool_calls(
                choice.content().unwrap_or(""),
                tool_calls.to_vec(),
            ));

            for tool_call in tool_calls {
                let tool_result =
                    execute_tool_call(tool_call, config.tavily_api_key.to_string()).await?;
                app.eventlog
                    .push(format!("tool call:  {} -> {}", tool_call.name(), tool_result));

                // Add tool response to session
                app.sessionmanager.add_message_to_current(Message::tool_response_named(
                    tool_call.id(),
                    tool_call.name(),
                    tool_result,
                ));
            }

            continue;
        } else {
            // Add final assistant message to session
            app.sessionmanager.add_message_to_current(Message::new(
                Role::Assistant,
                choice.content().unwrap_or(""),
            ));

            return Ok(format!(
                "{}Crust Agent:{}\n{}",
                ("=").repeat(50),
                ("=").repeat(50),
                choice.content().unwrap_or("")
            ));
        }
    }
    Ok("".to_string())
}

async fn execute_tool_call(
    tc: &ToolCall,
    tavily_api_key: String,
) -> Result<String, Box<dyn Error>> {
    //Web Search tool exec

    if tc.is_tool::<WebSearchParams>() {
        let params = tc.parse_params::<WebSearchParams>()?;
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
