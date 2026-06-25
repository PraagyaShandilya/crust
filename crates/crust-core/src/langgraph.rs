use std::{env, error::Error, path::PathBuf, time::Duration};

use crust_types::{
    LangGraphRegistry, LangGraphRunCommand, LangGraphRunRecord, LangGraphServer,
    LangGraphStreamEvent, Session,
};
use futures_util::StreamExt;
use openrouter_rs::types::Role;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{context, skills::load_markdown_skill, tools::resolve_tool_path};

const LANGGRAPH_REGISTRY_FILE: &str = "servers.json";

pub fn langgraph_root_dir() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".crust_langgraph")
}

pub fn langgraph_registry_path() -> PathBuf {
    langgraph_root_dir().join(LANGGRAPH_REGISTRY_FILE)
}

pub fn load_langgraph_registry() -> Result<LangGraphRegistry, String> {
    let path = langgraph_registry_path();
    if !path.exists() {
        return Ok(LangGraphRegistry::default());
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let registry: LangGraphRegistry = serde_json::from_str(&contents)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    registry.validate()?;
    Ok(registry)
}

pub fn save_langgraph_registry(registry: &LangGraphRegistry) -> Result<(), Box<dyn Error>> {
    registry.validate().map_err(std::io::Error::other)?;
    let path = langgraph_registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(registry)?)?;
    Ok(())
}

pub fn format_langgraph_registry(registry: &LangGraphRegistry) -> String {
    if registry.servers.is_empty() {
        return format!(
            "No LangGraph servers registered. Add one with `crust langgraph add <id> --url <base_url>`. Registry: {}",
            langgraph_registry_path().display()
        );
    }

    let mut lines = vec![format!(
        "Registered {} LangGraph server(s):",
        registry.servers.len()
    )];
    for server in &registry.servers {
        let assistant = server.assistant_id.as_deref().unwrap_or("none");
        let graph = server.default_graph.as_deref().unwrap_or("none");
        let auth = server.auth_env.as_deref().unwrap_or("none");
        lines.push(format!(
            "- {} ({}) -> {}\n  assistant_id: {assistant} | default_graph: {graph} | auth_env: {auth}",
            server.name, server.id, server.base_url
        ));
    }
    lines.join("\n")
}

pub fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn langgraph_timeout(server: &LangGraphServer) -> Duration {
    Duration::from_secs(server.timeout_secs.unwrap_or(30))
}

fn apply_langgraph_auth(
    request: reqwest::RequestBuilder,
    server: &LangGraphServer,
) -> Result<reqwest::RequestBuilder, Box<dyn Error + Send + Sync>> {
    let Some(auth_env) = server.auth_env.as_deref() else {
        return Ok(request);
    };
    let token = env::var(auth_env).map_err(|_| {
        std::io::Error::other(format!(
            "LangGraph auth env var `{auth_env}` is not set for `{}`",
            server.id
        ))
    })?;
    let header_name = match server.auth_header.as_deref() {
        Some(header) => HeaderName::from_bytes(header.as_bytes())?,
        None => AUTHORIZATION,
    };
    let header_value = if header_name == AUTHORIZATION {
        let scheme = server.auth_scheme.as_deref().unwrap_or("Bearer");
        HeaderValue::from_str(&format!("{scheme} {token}"))?
    } else {
        HeaderValue::from_str(&token)?
    };
    Ok(request.header(header_name, header_value))
}

pub async fn ping_langgraph_server(
    server: &LangGraphServer,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(langgraph_timeout(server))
        .build()?;
    let request = client.get(&server.base_url);
    let response = apply_langgraph_auth(request, server)?.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "LangGraph server `{}` responded with HTTP {status}",
            server.id
        )
        .into());
    }
    Ok(format!(
        "LangGraph server `{}` reachable at {} ({status})",
        server.id, server.base_url
    ))
}

fn langgraph_runs_dir() -> PathBuf {
    langgraph_root_dir().join("runs")
}

fn langgraph_raw_events_dir() -> PathBuf {
    langgraph_root_dir().join("raw_events")
}

fn langgraph_run_path(id: &str) -> PathBuf {
    langgraph_runs_dir().join(format!("{id}.json"))
}

fn save_langgraph_run_record(
    record: &LangGraphRunRecord,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    std::fs::create_dir_all(langgraph_runs_dir())?;
    std::fs::write(
        langgraph_run_path(&record.id),
        serde_json::to_string_pretty(record)?,
    )?;
    Ok(())
}

pub fn load_langgraph_run_record(
    id: &str,
) -> Result<LangGraphRunRecord, Box<dyn Error + Send + Sync>> {
    let path = langgraph_run_path(id);
    let contents = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(serde_json::from_str(&contents)?)
}

pub fn list_langgraph_run_records() -> Result<Vec<LangGraphRunRecord>, Box<dyn Error + Send + Sync>>
{
    let dir = langgraph_runs_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let contents = std::fs::read_to_string(&path)?;
        if let Ok(record) = serde_json::from_str::<LangGraphRunRecord>(&contents) {
            records.push(record);
        }
    }
    records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(records)
}

pub fn format_langgraph_runs(records: &[LangGraphRunRecord]) -> String {
    if records.is_empty() {
        return format!(
            "No LangGraph runs found. Run records are stored in `{}`.",
            langgraph_runs_dir().display()
        );
    }

    records
        .iter()
        .take(20)
        .map(|record| {
            format!(
                "{} [{}] server={} thread={} run={} - {}",
                record.id,
                record.status,
                record.server_id,
                record.thread_id,
                record.run_id.as_deref().unwrap_or("none"),
                context::truncate_middle(&record.input, 120)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_langgraph_run_result(record: &LangGraphRunRecord) -> String {
    let result = record
        .final_text
        .as_deref()
        .or(record.error.as_deref())
        .unwrap_or("No final text recorded.");
    format!(
        "LangGraph run {} [{}]\nserver: {}\nthread: {}\nrun: {}\nraw events: {}\n\n{}",
        record.id,
        record.status,
        record.server_id,
        record.thread_id,
        record.run_id.as_deref().unwrap_or("none"),
        record.raw_event_log_path,
        result
    )
}

pub fn langgraph_result_injection_summary(record: &LangGraphRunRecord) -> String {
    let result = record
        .final_text
        .as_deref()
        .or(record.error.as_deref())
        .unwrap_or("No final text recorded.");
    format!(
        "LangGraph run `{}` result summary:\nstatus: {}\nserver: {}\nthread: {}\nrun: {}\n\n{}",
        record.id,
        record.status,
        record.server_id,
        record.thread_id,
        record.run_id.as_deref().unwrap_or("none"),
        context::truncate_middle(result, 8_000)
    )
}

const LANGGRAPH_CONTEXT_SCOPES: &[&str] = &[
    "none",
    "summary",
    "recent",
    "files",
    "summary+recent",
    "summary+files",
    "full-allowed",
];

fn validate_langgraph_context_scope(scope: Option<&str>) -> Result<String, String> {
    let scope = scope.unwrap_or("summary+recent").trim();
    if LANGGRAPH_CONTEXT_SCOPES.contains(&scope) {
        Ok(scope.to_string())
    } else {
        Err(format!(
            "invalid context scope `{scope}`. Use one of: {}",
            LANGGRAPH_CONTEXT_SCOPES.join(", ")
        ))
    }
}

pub fn build_langgraph_handoff_payload(
    task: &str,
    session: Option<&Session>,
    context_scope: Option<&str>,
    files: &[String],
    skill_names: &[String],
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let context_scope =
        validate_langgraph_context_scope(context_scope).map_err(std::io::Error::other)?;
    let include_summary = matches!(
        context_scope.as_str(),
        "summary" | "summary+recent" | "summary+files" | "full-allowed"
    );
    let include_recent = matches!(
        context_scope.as_str(),
        "recent" | "summary+recent" | "full-allowed"
    );
    let include_files = matches!(
        context_scope.as_str(),
        "files" | "summary+files" | "full-allowed"
    );

    let session_value = session.map(|session| {
        let summary = if include_summary {
            session
                .summary
                .as_deref()
                .map(|summary| context::truncate_middle(summary, 8_000))
        } else {
            None
        };
        let recent_messages: Vec<Value> = if include_recent {
            session
                .messages
                .iter()
                .rev()
                .filter(|message| message.role != Role::System)
                .take(10)
                .map(|message| {
                    json!({
                        "role": format!("{}", message.role),
                        "content": context::truncate_middle(&context::content_to_compaction_text(&message.content, 4_000), 4_000),
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        } else {
            Vec::new()
        };
        json!({
            "id": session.id,
            "name": session.name,
            "summary": summary,
            "recent_messages": recent_messages,
        })
    });

    let mut file_values = Vec::new();
    if include_files {
        for file in files {
            let path = resolve_tool_path(file)?;
            let content = std::fs::read_to_string(&path)?;
            file_values.push(json!({
                "path": file,
                "resolved_path": path.to_string_lossy(),
                "content": context::truncate_middle(&content, 12_000),
            }));
        }
    }

    let mut skill_values = Vec::new();
    if !skill_names.is_empty() {
        for requested in skill_names {
            let skill = load_markdown_skill(requested).map_err(std::io::Error::other)?;
            skill_values.push(json!({
                "name": skill.name,
                "description": skill.description,
                "allowed_tools": skill.allowed_tools,
                "when_to_use": skill.when_to_use,
                "model": skill.model,
                "effort": skill.effort,
                "path": skill.path.to_string_lossy(),
                "content": skill.content.unwrap_or_default(),
            }));
        }
    }

    let cwd = env::current_dir()?;
    Ok(json!({
        "task": task,
        "context_scope": context_scope,
        "cwd": cwd.to_string_lossy(),
        "session": session_value,
        "files": file_values,
        "skills": skill_values,
        "source": "crust",
    }))
}

pub fn parse_langgraph_add_command(prompt: &str) -> Option<(&str, &str)> {
    let args = prompt.strip_prefix("/langgraph add ")?.trim();
    let mut parts = args.splitn(2, char::is_whitespace);
    let id = parts.next()?.trim();
    let url = parts.next()?.trim();
    if id.is_empty() || url.is_empty() {
        None
    } else {
        Some((id, url))
    }
}

pub fn parse_langgraph_run_command(prompt: &str) -> Option<LangGraphRunCommand> {
    let args = prompt.strip_prefix("/langgraph run ")?.trim();
    let mut parts = args.split_whitespace();
    let server_id = parts.next()?.trim().to_string();
    if server_id.is_empty() {
        return None;
    }

    let mut command = LangGraphRunCommand {
        server_id,
        ..Default::default()
    };
    let tokens: Vec<&str> = parts.collect();
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index] {
            "--context-scope" => {
                command.context_scope = Some(tokens.get(index + 1)?.to_string());
                index += 2;
            }
            "--file" => {
                command.files.push(tokens.get(index + 1)?.to_string());
                index += 2;
            }
            "--skill" => {
                command.skill_names.push(tokens.get(index + 1)?.to_string());
                index += 2;
            }
            _ => {
                command.input = tokens[index..].join(" ");
                break;
            }
        }
    }

    if command.input.trim().is_empty() {
        None
    } else {
        Some(command)
    }
}

pub fn parse_langgraph_single_arg_command<'a>(prompt: &'a str, prefix: &str) -> Option<&'a str> {
    let arg = prompt.strip_prefix(prefix)?.trim();
    if arg.is_empty() { None } else { Some(arg) }
}

pub fn parse_langgraph_result_command(prompt: &str) -> Option<(String, bool)> {
    let args = prompt.strip_prefix("/langgraph result")?.trim();
    if args.is_empty() {
        return None;
    }

    let mut run_id = None;
    let mut inject = false;
    for arg in args.split_whitespace() {
        if arg == "--inject" {
            inject = true;
        } else if run_id.is_none() {
            run_id = Some(arg.to_string());
        } else {
            return None;
        }
    }
    run_id.map(|run_id| (run_id, inject))
}

fn langgraph_url(server: &LangGraphServer, path: &str) -> String {
    format!("{}{}", server.base_url.trim_end_matches('/'), path)
}

fn extract_string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn extract_langgraph_run_id(value: &Value) -> Option<String> {
    extract_string_field(value, "run_id")
        .or_else(|| extract_string_field(value, "id"))
        .or_else(|| {
            value
                .get("metadata")
                .and_then(|metadata| extract_string_field(metadata, "run_id"))
        })
}

fn extract_langgraph_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    for key in ["final", "output", "result", "content", "message"] {
        if let Some(text) = extract_string_field(value, key) {
            return Some(text);
        }
    }
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        return messages.iter().rev().find_map(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    }
    None
}

fn push_langgraph_sse_event(
    events: &mut Vec<LangGraphStreamEvent>,
    current_event: &mut Option<String>,
    line: &str,
) {
    let line = line.trim_end_matches('\r');
    if line.is_empty() {
        return;
    }
    if let Some(event) = line.strip_prefix("event:").map(str::trim) {
        if !event.is_empty() {
            *current_event = Some(event.to_string());
        }
        return;
    }

    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return;
    };
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let value =
        serde_json::from_str::<Value>(data).unwrap_or_else(|_| Value::String(data.to_string()));
    events.push(LangGraphStreamEvent {
        event: current_event
            .clone()
            .unwrap_or_else(|| "message".to_string()),
        data: value,
    });
}

pub async fn create_langgraph_thread(
    server: &LangGraphServer,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(langgraph_timeout(server))
        .build()?;
    let request = client
        .post(langgraph_url(server, "/threads"))
        .json(&json!({}));
    let response = apply_langgraph_auth(request, server)?
        .send()
        .await?
        .error_for_status()?;
    let value: Value = response.json().await?;
    extract_string_field(&value, "thread_id")
        .or_else(|| extract_string_field(&value, "id"))
        .ok_or_else(|| {
            format!("LangGraph thread create response did not include thread_id: {value}").into()
        })
}

pub async fn stream_langgraph_run(
    server: &LangGraphServer,
    thread_id: &str,
    input: &str,
    handoff_payload: Option<Value>,
) -> Result<LangGraphRunRecord, Box<dyn Error + Send + Sync>> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    std::fs::create_dir_all(langgraph_raw_events_dir())?;
    let raw_event_path = langgraph_raw_events_dir().join(format!("{id}.json"));
    let assistant_id = server
        .assistant_id
        .as_deref()
        .or(server.default_graph.as_deref())
        .unwrap_or("agent");

    let mut input_payload = json!({
        "messages": [
            { "role": "user", "content": input }
        ]
    });
    if let Some(handoff_payload) = handoff_payload {
        input_payload["crust_handoff"] = handoff_payload;
    }

    let mut payload = json!({
        "assistant_id": assistant_id,
        "input": input_payload
    });
    if let Some(graph) = server.default_graph.as_deref() {
        payload["graph_id"] = Value::String(graph.to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(langgraph_timeout(server))
        .build()?;
    let request = client
        .post(langgraph_url(
            server,
            &format!("/threads/{thread_id}/runs/stream"),
        ))
        .json(&payload);
    let response = apply_langgraph_auth(request, server)?
        .send()
        .await?
        .error_for_status()?;

    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.contains("text/event-stream"));

    let mut events = Vec::new();
    if is_sse {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut current_event = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(newline_index) = buffer.find('\n') {
                let line = buffer[..newline_index].to_string();
                buffer.drain(..=newline_index);
                push_langgraph_sse_event(&mut events, &mut current_event, &line);
            }
        }
        if !buffer.is_empty() {
            push_langgraph_sse_event(&mut events, &mut current_event, &buffer);
        }
    } else {
        let text = response.text().await?;
        let data = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
        events.push(LangGraphStreamEvent {
            event: "response".to_string(),
            data,
        });
    }

    let run_id = events
        .iter()
        .find_map(|event| extract_langgraph_run_id(&event.data));
    let final_text = events
        .iter()
        .rev()
        .find_map(|event| extract_langgraph_text(&event.data));
    let status = "completed".to_string();
    std::fs::write(&raw_event_path, serde_json::to_string_pretty(&events)?)?;
    let updated_at = chrono::Utc::now().to_rfc3339();
    let record = LangGraphRunRecord {
        id,
        server_id: server.id.clone(),
        thread_id: thread_id.to_string(),
        run_id,
        status,
        input: input.to_string(),
        final_text,
        error: None,
        created_at: now,
        updated_at,
        raw_event_log_path: raw_event_path.to_string_lossy().to_string(),
        events,
    };
    save_langgraph_run_record(&record)?;
    Ok(record)
}

pub async fn run_langgraph_registered(
    server: &LangGraphServer,
    input: &str,
    handoff_payload: Option<Value>,
) -> Result<LangGraphRunRecord, Box<dyn Error + Send + Sync>> {
    let thread_id = create_langgraph_thread(server).await?;
    stream_langgraph_run(server, &thread_id, input, handoff_payload).await
}

pub async fn cancel_langgraph_run(
    record: &LangGraphRunRecord,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let registry = load_langgraph_registry().map_err(std::io::Error::other)?;
    let server = registry
        .find(&record.server_id)
        .ok_or_else(|| format!("LangGraph server `{}` not found", record.server_id))?;
    let run_id = record
        .run_id
        .as_deref()
        .ok_or("LangGraph run has no provider run_id to cancel")?;
    let client = reqwest::Client::builder()
        .timeout(langgraph_timeout(server))
        .build()?;
    let request = client.post(langgraph_url(
        server,
        &format!("/threads/{}/runs/{run_id}/cancel", record.thread_id),
    ));
    let response = apply_langgraph_auth(request, server)?.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("LangGraph cancel failed with HTTP {status}").into());
    }
    Ok(format!(
        "Cancel requested for LangGraph run `{}`",
        record.id
    ))
}
