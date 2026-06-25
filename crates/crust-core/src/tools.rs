use std::{
    env,
    error::Error,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use crust_types::{
    EditFileParams, LangGraphRunParams, ReadFileParams, ShellKind, ShellParams, ShellResult,
    WebSearchParams, WriteFileParams,
};
use openrouter_rs::types::ToolCall;
use serde_json::json;
use tavily::Tavily;
use tokio::time;

use crate::langgraph::*;

pub async fn run_shell_command(
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

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn resolve_tool_path(filename: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    let path = normalize_tool_path(filename);
    if path.as_os_str().is_empty() {
        return Err("tool filename cannot be empty".into());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(
            format!("tool path `{filename}` cannot contain parent directory segments").into(),
        );
    }

    let workspace = lexical_normalize_path(&env::current_dir()?);
    let resolved = if path.is_absolute() {
        lexical_normalize_path(&path)
    } else {
        lexical_normalize_path(&workspace.join(path))
    };

    if !resolved.starts_with(&workspace) {
        return Err(format!("tool path `{filename}` resolves outside the workspace").into());
    }

    Ok(resolved)
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

pub async fn execute_tool_call(tc: &ToolCall) -> Result<String, Box<dyn Error + Send + Sync>> {
    if tc.is_tool::<WebSearchParams>() {
        let params = tc.parse_params::<WebSearchParams>()?;
        crust_types::load_env();
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
            "answer": answer,
            "follow_up_questions": follow_up_questions,
            "results_titles": title,
            "results_urls": url,
            "results_contents": content,
        });

        return Ok(serde_json::to_string_pretty(&websearch)?);
    }

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
            "exitcode": result.exitcode,
            "output": result.output,
            "error": result.error,
        });
        return Ok(serde_json::to_string_pretty(&shellresults)?);
    }

    if tc.is_tool::<ReadFileParams>() {
        const DEFAULT_MAX_LINES: usize = 2_000;
        const DEFAULT_MAX_BYTES: usize = 50 * 1024;
        const MAX_LINES: usize = 20_000;

        let params = tc.parse_params::<ReadFileParams>()?;
        let filename = params.filename.clone();
        let filepath = resolve_tool_path(&filename)?;
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
            let line_bytes = line.len() + 1;
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
        let filepath = resolve_tool_path(&filename)?;
        std::fs::write(&filepath, content)?;

        let writerresults = json!({
            "filename": filename
        });
        return Ok(serde_json::to_string_pretty(&writerresults)?);
    }

    if tc.is_tool::<EditFileParams>() {
        let params = tc.parse_params::<EditFileParams>()?;
        if params.oldcontent.is_empty() {
            return Err("edit_file_tool oldcontent cannot be empty".into());
        }
        let filepath = resolve_tool_path(&params.filename)?;

        let mut buf = std::fs::read_to_string(&filepath)?;

        if let Some(offset) = buf.find(&params.oldcontent) {
            let end = offset + params.oldcontent.len();

            buf.replace_range(offset..end, &params.newcontent);

            std::fs::write(&filepath, buf)?;

            let editfileresults = json!({
                "err" : "false",
                "content" : "edit file activated"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        } else {
            let editfileresults = json!({
                "err" : "true",
                "content" : "Oldcontent not found"
            });

            return Ok(serde_json::to_string_pretty(&editfileresults)?);
        }
    }

    if tc.is_tool::<LangGraphRunParams>() {
        let params = tc.parse_params::<LangGraphRunParams>()?;
        let registry = load_langgraph_registry().map_err(std::io::Error::other)?;
        let base_server = registry.find(&params.server_id).ok_or_else(|| {
            std::io::Error::other(format!(
                "registered LangGraph server `{}` not found",
                params.server_id
            ))
        })?;
        let mut server = base_server.clone();
        if let Some(assistant_id) = params
            .assistant_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            server.assistant_id = Some(assistant_id.to_string());
        }
        if let Some(graph) = params
            .graph
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            server.default_graph = Some(graph.to_string());
        }

        let files = params.files.clone().unwrap_or_default();
        let skill_names = params.skill_names.clone().unwrap_or_default();
        let handoff = build_langgraph_handoff_payload(
            &params.input,
            None,
            params.context_scope.as_deref(),
            &files,
            &skill_names,
        )
        .map_err(|err| std::io::Error::other(err.to_string()))?;

        let thread_id = match params
            .thread_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            Some(thread_id) => thread_id.to_string(),
            None => create_langgraph_thread(&server)
                .await
                .map_err(|err| std::io::Error::other(err.to_string()))?,
        };
        let record = stream_langgraph_run(&server, &thread_id, &params.input, Some(handoff))
            .await
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        let result = json!({
            "id": record.id,
            "server_id": record.server_id,
            "thread_id": record.thread_id,
            "run_id": record.run_id,
            "status": record.status,
            "context_scope": params.context_scope,
            "skill_names": skill_names,
            "final_text": record.final_text,
            "raw_event_log_path": record.raw_event_log_path,
        });
        return Ok(serde_json::to_string_pretty(&result)?);
    }

    Ok("unhandled tool:{tc.name()}".to_string())
}
