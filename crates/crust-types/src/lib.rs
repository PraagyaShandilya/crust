use std::{env, error::Error, fmt};

use openrouter_rs::{
    Content, OpenRouterClient,
    api::chat::{ContentPart, Message},
    types::{Role, typed_tool::TypedTool},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use uuid::Uuid;

pub fn load_env() {
    dotenvy::dotenv().ok();
    dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            dotenvy::from_path(exe_dir.join(".env")).ok();
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    #[default]
    Idle,
    Done,
    Thinking,
    Tool,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggedAgentEvent {
    pub session_id: String,
    pub event: AgentEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    UserSubmitted {
        prompt: String,
    },
    Thinking {
        kind: String,
        text: String,
    },
    ToolCallStarted {
        name: String,
        args: String,
    },
    ToolCallFinished {
        name: String,
        result: String,
    },
    AssistantDelta {
        text: String,
    },
    AssistantFinal {
        text: String,
    },
    Error {
        message: String,
    },
    SystemNotice {
        message: String,
    },
    MaxStepsReached,
    SessionTitleUpdated {
        title: String,
    },
    ScopedAgentStarted {
        id: String,
        name: String,
        task: String,
        max_steps: u32,
    },
    ScopedAgentStep {
        id: String,
        step: u32,
    },
    ScopedAgentStatus {
        id: String,
        status: ScopedAgentStatus,
    },
    ScopedAgentLog {
        id: String,
        message: String,
    },
    ScopedAgentFinished {
        id: String,
        result: String,
    },
    ScopedAgentError {
        id: String,
        message: String,
    },
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedAgent {
    pub id: String,
    pub name: String,
    pub task: String,
    pub status: ScopedAgentStatus,
    pub current_step: u32,
    pub max_steps: u32,
    pub events: Vec<String>,
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ScopedAgentStatus {
    Pending,
    Running,
    Tool,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreKind {
    #[default]
    General,
    Learning,
    PairProgramming,
}

impl fmt::Display for ScopedAgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopedAgentStatus::Pending => write!(f, "Pending"),
            ScopedAgentStatus::Running => write!(f, "Running"),
            ScopedAgentStatus::Tool => write!(f, "Tool"),
            ScopedAgentStatus::Done => write!(f, "Done"),
            ScopedAgentStatus::Error => write!(f, "Error"),
            ScopedAgentStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub max_agent_steps: u32,
    pub modelname: String,
}

impl Config {
    pub fn new() -> Self {
        load_env();

        let max_agent_steps = env::var("MAX_AGENT_STEPS")
            .unwrap_or("10".to_string())
            .parse::<u32>()
            .unwrap_or(10);

        let modelname = env::var("OPENROUTER_MAIN_MODEL")
            .unwrap_or_else(|_| "moonshotai/kimi-latest".to_string())
            .to_string();

        Config {
            max_agent_steps,
            modelname,
        }
    }

    pub fn client_builder(&self) -> Result<OpenRouterClient, Box<dyn Error + Send + Sync>> {
        load_env();
        let api_key = env::var("OPENROUTER_API_KEY")
            .map_err(|_| std::io::Error::other("OPENROUTER_API_KEY must be set"))?;
        let client = OpenRouterClient::builder()
            .api_key(api_key)
            .build()
            .map_err(|err| {
                std::io::Error::other(format!("OpenRouter client build failed: {err}"))
            })?;
        Ok(client)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub edited_at: String,
    pub messages: Vec<Message>,
    pub sysprompt: Message,
    pub config: Config,
    #[serde(default)]
    pub core_profile: CoreKind,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub compacted_until: usize,
    #[serde(default)]
    pub latest_prompt_tokens: u32,
    #[serde(default)]
    pub latest_completion_tokens: u32,
    #[serde(default)]
    pub latest_total_tokens: u32,
    #[serde(default)]
    pub cumulative_prompt_tokens: u64,
    #[serde(default)]
    pub cumulative_completion_tokens: u64,
    #[serde(default)]
    pub cumulative_total_tokens: u64,
}

impl Session {
    pub fn new(name: String, sysprompt: &str) -> Self {
        let sysprompt: Message = Message::new(Role::System, sysprompt);
        let messages = vec![sysprompt.clone()];
        let now = chrono::Utc::now().to_rfc3339();
        let config = Config::new();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            created_at: now.clone(),
            edited_at: now,
            messages,
            sysprompt,
            config,
            core_profile: CoreKind::default(),
            summary: None,
            compacted_until: 0,
            latest_prompt_tokens: 0,
            latest_completion_tokens: 0,
            latest_total_tokens: 0,
            cumulative_prompt_tokens: 0,
            cumulative_completion_tokens: 0,
            cumulative_total_tokens: 0,
        }
    }

    pub fn touch(&mut self) {
        self.edited_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
        self.touch();
    }

    pub fn record_usage(&mut self, prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) {
        self.latest_prompt_tokens = prompt_tokens;
        self.latest_completion_tokens = completion_tokens;
        self.latest_total_tokens = total_tokens;
        self.cumulative_prompt_tokens += u64::from(prompt_tokens);
        self.cumulative_completion_tokens += u64::from(completion_tokens);
        self.cumulative_total_tokens += u64::from(total_tokens);
        self.touch();
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.messages.push(self.sysprompt.clone());
        self.summary = None;
        self.compacted_until = 0;
        self.touch();
    }

    pub fn compact_to_recent(&mut self, min_recent_messages: usize) -> Result<usize, String> {
        let cutoff = self.messages.len().saturating_sub(min_recent_messages);
        if cutoff <= self.compacted_until + 1 {
            return Err("Not enough uncompacted history to compact.".to_string());
        }

        let mut summary = String::new();
        if let Some(existing_summary) = self
            .summary
            .as_deref()
            .filter(|summary| !summary.trim().is_empty())
        {
            summary.push_str("Previous compacted summary:\n");
            summary.push_str(existing_summary.trim());
            summary.push_str("\n\nNewly compacted transcript notes:\n");
        } else {
            summary.push_str("Compacted transcript notes:\n");
        }

        for (index, message) in self
            .messages
            .iter()
            .enumerate()
            .skip(self.compacted_until)
            .take(cutoff.saturating_sub(self.compacted_until))
        {
            if message.role == Role::System {
                continue;
            }

            let content = content_to_compaction_text(&message.content, 1_500);
            summary.push_str(&format!(
                "\n[{}] {}: {}",
                index,
                message.role,
                content.trim()
            ));

            if let Some(tool_calls) = &message.tool_calls {
                let calls = serde_json::to_string(tool_calls).unwrap_or_default();
                summary.push_str("\n  tool_calls: ");
                summary.push_str(&truncate_middle(&calls, 1_000));
            }
        }

        self.summary = Some(truncate_middle(&summary, 40_000));
        self.compacted_until = cutoff;
        self.touch();
        Ok(cutoff)
    }

    pub fn get_config(&self) -> Config {
        self.config.clone()
    }
}

fn content_to_compaction_text(content: &Content, limit: usize) -> String {
    let text = match content {
        Content::Text(text) => text.clone(),
        Content::Parts(parts) => parts
            .iter()
            .map(|part| match part {
                ContentPart::Text { text, .. } => text.clone(),
                _ => {
                    serde_json::to_string(part).unwrap_or_else(|_| "[non-text content]".to_string())
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };

    truncate_middle(&text, limit)
}

fn truncate_middle(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 64 {
        return text.chars().take(max_chars).collect();
    }

    let marker = "\n... [truncated] ...\n";
    let available = max_chars.saturating_sub(marker.len());
    let head_chars = available / 2;
    let tail_chars = available.saturating_sub(head_chars);

    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    format!("{head}{marker}{tail}")
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SpacesRegistry {
    #[serde(default)]
    pub spaces: Vec<CrustSpace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrustSpace {
    pub id: String,
    pub name: String,
    pub session_id: String,
    pub cwd: String,
    pub status: SpaceStatus,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpaceStatus {
    Idle,
    Running,
    Stopped,
    Failed,
}

impl fmt::Display for SpaceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpaceStatus::Idle => write!(f, "idle"),
            SpaceStatus::Running => write!(f, "running"),
            SpaceStatus::Stopped => write!(f, "stopped"),
            SpaceStatus::Failed => write!(f, "failed"),
        }
    }
}

impl SpacesRegistry {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for space in &self.spaces {
            if space.id.trim().is_empty() {
                return Err("space id cannot be empty".to_string());
            }
            if space.name.trim().is_empty() {
                return Err(format!("space `{}` name cannot be empty", space.id));
            }
            if space.cwd.trim().is_empty() {
                return Err(format!("space `{}` cwd cannot be empty", space.id));
            }
            if !ids.insert(space.id.to_ascii_lowercase()) {
                return Err(format!("duplicate space id `{}`", space.id));
            }
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&CrustSpace> {
        self.spaces
            .iter()
            .find(|space| space.id.eq_ignore_ascii_case(id.trim()))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut CrustSpace> {
        self.spaces
            .iter_mut()
            .find(|space| space.id.eq_ignore_ascii_case(id.trim()))
    }

    pub fn upsert(&mut self, space: CrustSpace) {
        if let Some(existing) = self.find_mut(&space.id) {
            *existing = space;
        } else {
            self.spaces.push(space);
            self.spaces.sort_by(|left, right| {
                left.id
                    .to_ascii_lowercase()
                    .cmp(&right.id.to_ascii_lowercase())
            });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProtocolMessage {
    pub protocol_version: u32,
    pub message_id: String,
    pub space_id: String,
    pub task_id: String,
    pub message_type: AgentProtocolMessageType,
    pub correlation_id: String,
    pub timestamp: String,
    pub payload: AgentProtocolPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentProtocolMessageType {
    ContextHandoff,
    ResultReturn,
    Control,
    Status,
    Progress,
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentProtocolPayload {
    ContextHandoff(SpaceContextHandoff),
    ResultReturn(SpaceResultReturn),
    Control(SpaceControl),
    Status(SpaceStatusUpdate),
    Progress(SpaceProgress),
    Heartbeat(SpaceHeartbeat),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceContextHandoff {
    pub task: String,
    pub cwd: String,
    #[serde(default)]
    pub compact_summary: Option<String>,
    #[serde(default)]
    pub recent_messages: Vec<Value>,
    #[serde(default)]
    pub selected_files: Vec<Value>,
    #[serde(default)]
    pub skills: Vec<Value>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceResultReturn {
    pub final_summary: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub validation: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub raw_log_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceControl {
    pub command: SpaceControlCommand,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpaceControlCommand {
    Cancel,
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceStatusUpdate {
    pub status: SpaceStatus,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceProgress {
    pub message: String,
    #[serde(default)]
    pub step: Option<u32>,
    #[serde(default)]
    pub total_steps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceHeartbeat {
    pub status: SpaceStatus,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LangGraphRegistry {
    #[serde(default)]
    pub servers: Vec<LangGraphServer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangGraphServer {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub assistant_id: Option<String>,
    #[serde(default)]
    pub default_graph: Option<String>,
    #[serde(default)]
    pub auth_env: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub auth_scheme: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl LangGraphRegistry {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for server in &self.servers {
            if server.id.trim().is_empty() {
                return Err("LangGraph server id cannot be empty".to_string());
            }
            if server.name.trim().is_empty() {
                return Err(format!(
                    "LangGraph server `{}` name cannot be empty",
                    server.id
                ));
            }
            if server.base_url.trim().is_empty() {
                return Err(format!(
                    "LangGraph server `{}` base_url cannot be empty",
                    server.id
                ));
            }
            let normalized = server.id.to_ascii_lowercase();
            if !ids.insert(normalized) {
                return Err(format!("duplicate LangGraph server id `{}`", server.id));
            }
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&LangGraphServer> {
        let id = id.trim();
        self.servers
            .iter()
            .find(|server| server.id.eq_ignore_ascii_case(id))
    }

    pub fn upsert(&mut self, server: LangGraphServer) {
        if let Some(existing) = self
            .servers
            .iter_mut()
            .find(|existing| existing.id.eq_ignore_ascii_case(&server.id))
        {
            *existing = server;
        } else {
            self.servers.push(server);
            self.servers.sort_by(|left, right| {
                left.id
                    .to_ascii_lowercase()
                    .cmp(&right.id.to_ascii_lowercase())
            });
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.servers.len();
        self.servers
            .retain(|server| !server.id.eq_ignore_ascii_case(id.trim()));
        self.servers.len() != before
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangGraphStreamEvent {
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangGraphRunRecord {
    pub id: String,
    pub server_id: String,
    pub thread_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    pub status: String,
    pub input: String,
    #[serde(default)]
    pub final_text: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub raw_event_log_path: String,
    #[serde(default)]
    pub events: Vec<LangGraphStreamEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct LangGraphRunCommand {
    pub server_id: String,
    pub input: String,
    pub context_scope: Option<String>,
    pub files: Vec<String>,
    pub skill_names: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub provider: &'static str,
    pub context_window: u32,
    pub max_tokens: u32,
    pub input_modalities: &'static [&'static str],
    pub output_modalities: &'static [&'static str],
    pub tokenizer: &'static str,
    pub reasoning: bool,
    pub tools: bool,
    pub input_cost_per_million: f64,
    pub output_cost_per_million: f64,
    pub cache_read_cost_per_million: f64,
    pub cache_write_cost_per_million: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    PowerShell,
    Cmd,
}

impl ShellKind {
    pub fn from_env() -> Result<Self, String> {
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

    pub fn default_for_current_os() -> Self {
        Self::default_for_os(cfg!(windows))
    }

    fn default_for_os(is_windows: bool) -> Self {
        if is_windows {
            Self::PowerShell
        } else {
            Self::Bash
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Bash => "Bash",
            Self::PowerShell => "PowerShell",
            Self::Cmd => "cmd.exe",
        }
    }

    pub fn command(self, command: &str) -> Command {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellResult {
    pub exitcode: i32,
    pub output: String,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct WebSearchParams {
    pub query: String,
    pub max_results: usize,
}

impl TypedTool for WebSearchParams {
    fn name() -> &'static str {
        "web_search_tool"
    }

    fn description() -> &'static str {
        "Look up the web for a query using search"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ShellParams {
    pub timeout: u32,
    pub command: String,
}

impl TypedTool for ShellParams {
    fn name() -> &'static str {
        "shell_calling_tool"
    }

    fn description() -> &'static str {
        "Run a command in the configured workspace shell"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ReadFileParams {
    pub filename: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

impl TypedTool for ReadFileParams {
    fn name() -> &'static str {
        "read_file_tool"
    }
    fn description() -> &'static str {
        "Read a file by filename. For normal files, omit offset and limit to read up to 2000 lines or 50KB at once. offset is a 1-indexed line number and limit is a line count. Use offset/limit only for large files; continue from next_offset if returned."
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct WriteFileParams {
    pub filename: String,
    pub content: String,
}

impl TypedTool for WriteFileParams {
    fn name() -> &'static str {
        "write_file_tool"
    }
    fn description() -> &'static str {
        "Write a file using its file name and a content string"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct EditFileParams {
    pub filename: String,
    pub oldcontent: String,
    pub newcontent: String,
}

impl TypedTool for EditFileParams {
    fn name() -> &'static str {
        "edit_file_tool"
    }
    fn description() -> &'static str {
        "Edit a file using its file finding and editing an oldcontent and replacing it with newcontent"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct LangGraphRunParams {
    pub server_id: String,
    pub input: String,
    pub assistant_id: Option<String>,
    pub graph: Option<String>,
    pub thread_id: Option<String>,
    pub files: Option<Vec<String>>,
    pub skill_names: Option<Vec<String>>,
    pub context_scope: Option<String>,
}

impl TypedTool for LangGraphRunParams {
    fn name() -> &'static str {
        "langgraph_run_tool"
    }
    fn description() -> &'static str {
        "Run a registered LangGraph dev-server workflow by server_id. Use only registered server IDs; arbitrary URLs are not accepted. input is the task/prompt to hand off. Optional assistant_id, graph, thread_id, files, skill_names, and context_scope fields control the handoff payload."
    }
}
