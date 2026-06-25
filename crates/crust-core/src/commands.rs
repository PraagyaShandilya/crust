pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/exit", "Exit the application"),
    ("/new <session_name>", "Create a new session"),
    ("/clear", "Clear current session history"),
    ("/context", "Show context status"),
    ("/compact", "Compact session history"),
    ("/agent <name> <task>", "Run a scoped child agent"),
    ("/agents", "List scoped agents"),
    ("/agent-cancel <name>", "Cancel a scoped agent"),
    ("/agent-result <name>", "Show scoped agent result"),
    ("/skills", "List loaded markdown skills"),
    ("/skill <name> [args]", "Invoke a markdown skill explicitly"),
    ("/langgraph list", "List registered LangGraph dev servers"),
    (
        "/langgraph add <id> <url>",
        "Register a LangGraph dev server",
    ),
    (
        "/langgraph run <id> <input>",
        "Run a LangGraph dev-server workflow",
    ),
    ("/langgraph runs", "List persisted LangGraph runs"),
    ("/langgraph result <run_id>", "Show a LangGraph run result"),
    ("/langgraph cancel <run_id>", "Cancel a LangGraph run"),
    ("/spaces", "List Crust spaces"),
    ("/space-create <id>", "Create a Crust space"),
    ("/space-attach <id>", "Show Crust space details"),
    ("/space-stop <id>", "Stop a Crust space"),
    ("/delete <session_name>", "Delete a session"),
    ("/switch <session_name>", "Switch to a session"),
];

pub fn get_filtered_commands(filter: &str) -> Vec<(&'static str, &'static str)> {
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
