use crust_types::{CoreKind, ShellKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Autonomous,
    SuggestAndConfirm,
}

#[derive(Debug, Clone)]
pub struct CoreProfile {
    pub kind: CoreKind,
    pub display_name: &'static str,
    pub description: &'static str,
    pub system_prompt: String,
    pub interaction_mode: InteractionMode,
    pub default_model: Option<&'static str>,
}

pub fn core_profile(kind: CoreKind) -> CoreProfile {
    match kind {
        CoreKind::General => CoreProfile {
            kind,
            display_name: "General",
            description: "Autonomous general-purpose coding agent.",
            system_prompt: general_system_prompt(),
            interaction_mode: InteractionMode::Autonomous,
            default_model: None,
        },
        CoreKind::Learning => CoreProfile {
            kind,
            display_name: "Learning",
            description: "Autonomous agent that explains decisions while helping you learn.",
            system_prompt: learning_system_prompt(),
            interaction_mode: InteractionMode::Autonomous,
            default_model: Some("anthropic/claude-sonnet-4"),
        },
        CoreKind::PairProgramming => CoreProfile {
            kind,
            display_name: "Pair Programming",
            description: "Suggests tool use and waits for confirmation before executing.",
            system_prompt: pair_programming_system_prompt(),
            interaction_mode: InteractionMode::SuggestAndConfirm,
            default_model: Some("anthropic/claude-sonnet-4"),
        },
    }
}

pub fn default_system_prompt() -> String {
    core_profile(CoreKind::General).system_prompt
}

fn shell_display_name() -> &'static str {
    ShellKind::from_env()
        .unwrap_or_else(|_| ShellKind::default_for_current_os())
        .display_name()
}

fn general_system_prompt() -> String {
    format!(
        "You are an AI agent given tools to help people. Use the shell tool to run {shell_name} commands in the workspace, the read/write/edit tools for files when you know filenames, and the web search tool to interface with the web.",
        shell_name = shell_display_name()
    )
}

fn learning_system_prompt() -> String {
    format!(
        "You are a learning-focused coding agent. Help the user solve the task while teaching the reasoning behind important choices. Use the shell tool to run {shell_name} commands in the workspace, the read/write/edit tools for files when you know filenames, and the web search tool when useful. Prefer concise explanations tied to the current code over generic tutorials.",
        shell_name = shell_display_name()
    )
}

fn pair_programming_system_prompt() -> String {
    format!(
        "You are a pair programming coding agent. Work collaboratively, explain proposed changes before risky actions, and keep the user in control. Use the shell tool to run {shell_name} commands in the workspace, the read/write/edit tools for files when you know filenames, and the web search tool when useful. In suggest-and-confirm mode, tool calls are proposed for human approval before execution.",
        shell_name = shell_display_name()
    )
}
