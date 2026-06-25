use crust_types::{
    AgentProtocolMessage, AgentProtocolMessageType, AgentProtocolPayload, CrustSpace,
    SpaceContextHandoff, SpaceControlCommand, SpaceStatus, SpacesRegistry,
};
use std::{
    env,
    error::Error,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const SPACES_REGISTRY_FILE: &str = "spaces.json";
pub const AGENT_PROTOCOL_VERSION: u32 = 1;

pub fn spaces_root_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".crust_spaces")
}

pub fn spaces_registry_path() -> PathBuf {
    spaces_root_dir().join(SPACES_REGISTRY_FILE)
}

pub fn load_spaces_registry() -> Result<SpacesRegistry, String> {
    let path = spaces_registry_path();
    if !path.exists() {
        return Ok(SpacesRegistry::default());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let registry: SpacesRegistry = serde_json::from_str(&contents)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    registry.validate()?;
    Ok(registry)
}

pub fn save_spaces_registry(registry: &SpacesRegistry) -> Result<(), Box<dyn Error>> {
    registry.validate().map_err(std::io::Error::other)?;
    let path = spaces_registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(registry)?)?;
    Ok(())
}

pub fn format_spaces_registry(registry: &SpacesRegistry) -> String {
    if registry.spaces.is_empty() {
        return format!(
            "No Crust spaces registered. Create one with `crust spaces create <id>`. Registry: {}",
            spaces_registry_path().display()
        );
    }

    registry
        .spaces
        .iter()
        .map(|space| {
            format!(
                "{} [{}] session={} cwd={} task={} pid={}",
                space.id,
                space.status,
                space.session_id,
                space.cwd,
                space.task.as_deref().unwrap_or("none"),
                space
                    .process_id
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "none".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn validate_agent_protocol_message(
    message: &AgentProtocolMessage,
    current_status: Option<&SpaceStatus>,
) -> Result<(), String> {
    if message.protocol_version != AGENT_PROTOCOL_VERSION {
        return Err(format!(
            "unsupported AgentProtocol version `{}`; expected `{AGENT_PROTOCOL_VERSION}`",
            message.protocol_version
        ));
    }
    for (field, value) in [
        ("message_id", message.message_id.as_str()),
        ("space_id", message.space_id.as_str()),
        ("task_id", message.task_id.as_str()),
        ("correlation_id", message.correlation_id.as_str()),
        ("timestamp", message.timestamp.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("AgentProtocol missing required field `{field}`"));
        }
    }

    match (&message.message_type, &message.payload) {
        (
            AgentProtocolMessageType::ContextHandoff,
            AgentProtocolPayload::ContextHandoff(payload),
        ) => {
            if payload.task.trim().is_empty() {
                return Err("context handoff task cannot be empty".to_string());
            }
            if payload.cwd.trim().is_empty() {
                return Err("context handoff cwd cannot be empty".to_string());
            }
        }
        (AgentProtocolMessageType::ResultReturn, AgentProtocolPayload::ResultReturn(payload)) => {
            if payload.final_summary.trim().is_empty() {
                return Err("result return final_summary cannot be empty".to_string());
            }
        }
        (AgentProtocolMessageType::Control, AgentProtocolPayload::Control(payload)) => {
            if let (Some(SpaceStatus::Stopped), SpaceControlCommand::Resume) =
                (current_status, &payload.command)
            {
                return Err("cannot resume a stopped space".to_string());
            }
        }
        (AgentProtocolMessageType::Status, AgentProtocolPayload::Status(_))
        | (AgentProtocolMessageType::Progress, AgentProtocolPayload::Progress(_))
        | (AgentProtocolMessageType::Heartbeat, AgentProtocolPayload::Heartbeat(_)) => {}
        _ => {
            return Err(format!(
                "AgentProtocol message_type `{:?}` does not match payload variant",
                message.message_type
            ));
        }
    }
    Ok(())
}

pub fn space_ipc_dir(space_id: &str) -> PathBuf {
    spaces_root_dir().join(space_id).join("ipc")
}

pub fn space_inbox_path(space_id: &str) -> PathBuf {
    space_ipc_dir(space_id).join("inbox.jsonl")
}

pub fn space_outbox_path(space_id: &str) -> PathBuf {
    space_ipc_dir(space_id).join("outbox.jsonl")
}

pub fn append_agent_protocol_message(
    path: &Path,
    message: &AgentProtocolMessage,
    current_status: Option<&SpaceStatus>,
) -> Result<(), Box<dyn Error>> {
    validate_agent_protocol_message(message, current_status).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let line = serde_json::to_string(message)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

pub fn build_space_context_handoff_message(space: &CrustSpace, task: &str) -> AgentProtocolMessage {
    let task_id = space
        .task_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    AgentProtocolMessage {
        protocol_version: AGENT_PROTOCOL_VERSION,
        message_id: Uuid::new_v4().to_string(),
        space_id: space.id.clone(),
        task_id,
        message_type: AgentProtocolMessageType::ContextHandoff,
        correlation_id: Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        payload: AgentProtocolPayload::ContextHandoff(SpaceContextHandoff {
            task: task.to_string(),
            cwd: space.cwd.clone(),
            compact_summary: None,
            recent_messages: Vec::new(),
            selected_files: Vec::new(),
            skills: Vec::new(),
            constraints: vec![
                "Use only the provided task context unless told otherwise.".to_string(),
            ],
        }),
    }
}
