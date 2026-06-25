use crate::models_generated;
use crust_types::{Config, Session};
use openrouter_rs::{
    Content,
    api::chat::{ContentPart, Message},
    types::Role,
};

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 128_000;

#[derive(Debug, Clone)]
pub struct ContextBuilder {
    pub max_context_ratio_before_compaction: f64,
    pub target_context_ratio_after_compaction: f64,
    pub max_tool_response_chars: usize,
    pub min_recent_messages: usize,
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self {
            max_context_ratio_before_compaction: 0.70,
            target_context_ratio_after_compaction: 0.45,
            max_tool_response_chars: 12_000,
            min_recent_messages: 12,
        }
    }
}

impl ContextBuilder {
    pub fn build_context(&self, session: &Session, config: &Config) -> Vec<Message> {
        let context_window = model_context_window_tokens(&config.modelname);
        let target_char_budget =
            ((context_window as f64) * self.target_context_ratio_after_compaction * 4.0) as usize;

        let mut context = Vec::new();
        context.push(session.sysprompt.clone());

        if let Some(summary) = session
            .summary
            .as_deref()
            .filter(|summary| !summary.trim().is_empty())
        {
            context.push(Message::new(
                Role::System,
                format!(
                    "Prior compacted conversation summary. Treat this as durable context, not a new user request.\n\n{summary}"
                ),
            ));
        }

        let message_count = session.messages.len();
        let min_recent_start = message_count.saturating_sub(self.min_recent_messages);
        let start_index = if session.summary.is_some() || session.compacted_until > 0 {
            session.compacted_until.min(min_recent_start)
        } else {
            0
        };

        for message in session.messages.iter().skip(start_index) {
            if message.role == Role::System {
                continue;
            }
            context.push(self.prepare_message_for_context(message));
        }

        self.drop_oldest_until_under_budget(context, target_char_budget)
    }

    pub fn context_window_tokens(&self, config: &Config) -> usize {
        model_context_window_tokens(&config.modelname)
    }

    pub fn context_ratio_from_api_usage(&self, prompt_tokens_sent: u32, config: &Config) -> f64 {
        context_usage_ratio(prompt_tokens_sent, self.context_window_tokens(config))
    }

    pub fn should_compact_from_api_usage(&self, prompt_tokens_sent: u32, config: &Config) -> bool {
        self.context_ratio_from_api_usage(prompt_tokens_sent, config)
            >= self.max_context_ratio_before_compaction
    }

    pub fn estimate_context_tokens(&self, context: &[Message]) -> usize {
        estimate_tokens_for_messages(context)
    }

    pub fn estimated_context_ratio(&self, context: &[Message], config: &Config) -> f64 {
        let window = self.context_window_tokens(config);
        if window == 0 {
            0.0
        } else {
            self.estimate_context_tokens(context) as f64 / window as f64
        }
    }

    pub fn should_compact_estimated_context(&self, context: &[Message], config: &Config) -> bool {
        self.estimated_context_ratio(context, config) >= self.max_context_ratio_before_compaction
    }

    fn prepare_message_for_context(&self, message: &Message) -> Message {
        if message.role == Role::Tool {
            truncate_message_text(message, self.max_tool_response_chars)
        } else {
            message.clone()
        }
    }

    fn drop_oldest_until_under_budget(
        &self,
        mut context: Vec<Message>,
        target_char_budget: usize,
    ) -> Vec<Message> {
        if target_char_budget == 0 {
            return context;
        }

        // Keep system prompt + optional compacted summary + a minimum recent tail.
        // Remove whole conversation units rather than individual messages so
        // assistant tool_calls and their corresponding tool responses do not
        // become orphaned in the API context.
        while message_char_len_sum(&context) > target_char_budget
            && context.len() > self.min_recent_messages + 1
        {
            if !remove_oldest_non_system_unit(&mut context) {
                break;
            }
        }

        context
    }
}

fn remove_oldest_non_system_unit(context: &mut Vec<Message>) -> bool {
    let Some(start) = context
        .iter()
        .position(|message| message.role != Role::System)
    else {
        return false;
    };

    let mut end = start + 1;
    while end < context.len() {
        if context[end].role == Role::System {
            end += 1;
            continue;
        }

        // User messages begin a new conversation turn. Stop before the next
        // user so the removed range is one complete old turn, or an orphaned
        // assistant/tool prefix if the context already started mid-turn.
        if context[end].role == Role::User {
            break;
        }

        end += 1;
    }

    context.drain(start..end);
    true
}

pub fn model_context_window_tokens(modelname: &str) -> usize {
    models_generated::get_openrouter_model(modelname)
        .map(|model| model.context_window as usize)
        .filter(|tokens| *tokens > 0)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
}

pub fn context_usage_ratio(prompt_tokens_sent: u32, context_window_tokens: usize) -> f64 {
    if context_window_tokens == 0 {
        0.0
    } else {
        f64::from(prompt_tokens_sent) / context_window_tokens as f64
    }
}

pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
    message_char_len_sum(messages).div_ceil(4)
}

pub fn message_char_len_sum(messages: &[Message]) -> usize {
    messages.iter().map(message_char_len).sum()
}

pub fn message_char_len(message: &Message) -> usize {
    let tool_calls_len = message
        .tool_calls
        .as_ref()
        .and_then(|tool_calls| serde_json::to_string(tool_calls).ok())
        .map_or(0, |serialized| serialized.len());

    role_len(&message.role) + content_char_len(&message.content) + tool_calls_len
}

pub fn content_char_len(content: &Content) -> usize {
    match content {
        Content::Text(text) => text.len(),
        Content::Parts(parts) => parts
            .iter()
            .map(|part| match part {
                ContentPart::Text { text, .. } => text.len(),
                _ => serde_json::to_string(part).map_or(0, |serialized| serialized.len()),
            })
            .sum(),
    }
}

pub fn content_to_compaction_text(content: &Content, limit: usize) -> String {
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

pub fn truncate_message_text(message: &Message, max_chars: usize) -> Message {
    let mut truncated = message.clone();
    if let Content::Text(text) = &message.content {
        if text.len() > max_chars {
            truncated.content = Content::Text(format!(
                "[Tool output truncated from {} chars to {} chars for model context]\n{}",
                text.len(),
                max_chars,
                truncate_middle(text, max_chars),
            ));
        }
    }
    truncated
}

pub fn truncate_middle(text: &str, max_chars: usize) -> String {
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

fn role_len(role: &Role) -> usize {
    match role {
        Role::System => 6,
        Role::Developer => 9,
        Role::User => 4,
        Role::Assistant => 9,
        Role::Tool => 4,
    }
}
