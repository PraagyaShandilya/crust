use std::{env, time::Duration};

use crust_types::{Config, Session};

use crate::ContextBuilder;

pub fn append_delta(existing: &mut String, delta: &str) {
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

pub fn format_token_count(tokens: usize) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

pub fn env_duration_secs_or_default(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(
        env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(default_secs),
    )
}

pub fn context_pressure_message(
    context_builder: &ContextBuilder,
    config: &Config,
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

pub fn format_current_session_title(session: &Session) -> String {
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

fn compact_log_payload(value: &str, max_chars: usize) -> String {
    let compact = serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|json| serde_json::to_string(&json).ok())
        .unwrap_or_else(|| value.trim().replace('\n', " "));

    if compact.len() <= max_chars {
        compact
    } else {
        format!(
            "{} [shown {max_chars}/{} chars]",
            crate::truncate_middle(&compact, max_chars),
            compact.len()
        )
    }
}

pub fn compact_tool_call_text(name: &str, args: &str) -> String {
    format!("tool call: {name} args={}", compact_log_payload(args, 600))
}

pub fn compact_tool_result_text(name: &str, result: &str) -> String {
    format!(
        "tool result: {name} -> {}",
        compact_log_payload(result, 1_000)
    )
}
