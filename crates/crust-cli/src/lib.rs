use std::{env, error::Error};

use crust_core::{langgraph::*, skills::*, spaces::*};
use crust_types::{CrustSpace, LangGraphServer, SpaceStatus};
use uuid::Uuid;

pub async fn run_cli(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("help" | "--help" | "-h") => {
            print_cli_help();
            Ok(())
        }
        Some("langgraph") => run_langgraph_cli(&args[1..]).await,
        Some("skills") => run_skills_cli(&args[1..]),
        Some("skill") => run_skill_cli(&args[1..]),
        Some("spaces") => run_spaces_cli(&args[1..]),
        Some(command) => Err(format!(
            "unknown command `{command}`. Run `crust help` for available commands."
        )
        .into()),
        None => Ok(()),
    }
}

fn print_cli_help() {
    println!(
        "crust\n\nUsage:\n  crust                 Open the interactive TUI\n  crust help            Show this help\n  crust skills list\n  crust skills show <name>\n  crust skill <name> [args]\n  crust spaces list\n  crust spaces create <id> [--cwd <path>] [--task <task>]\n  crust spaces spawn <id> [--cwd <path>] <task>\n  crust spaces attach <id>\n  crust spaces stop <id>\n  crust langgraph add <id> --url <base_url> [--name <name>] [--assistant-id <id>] [--default-graph <graph>] [--auth-env <var>] [--auth-header <header>] [--auth-scheme <scheme>] [--timeout-secs <secs>]\n  crust langgraph list\n  crust langgraph remove <id>\n  crust langgraph ping <id>"
    );
}

fn run_spaces_cli(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let registry = load_spaces_registry().map_err(std::io::Error::other)?;
            println!("{}", format_spaces_registry(&registry));
            Ok(())
        }
        Some("create") => cli_spaces_create(&args[1..]),
        Some("spawn") => cli_spaces_spawn(&args[1..]),
        Some("attach") => cli_spaces_attach(&args[1..]),
        Some("stop") => cli_spaces_stop(&args[1..]),
        Some(command) => Err(format!(
            "unknown spaces command `{command}`. Use list, create, attach, or stop."
        )
        .into()),
    }
}

fn parse_space_spawn_args(args: &[String]) -> Result<(String, String, String), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust spaces spawn <id> [--cwd <path>] <task>".into());
    };
    if id.starts_with('-') || id.trim().is_empty() {
        return Err("space id cannot be empty or start with `-`".into());
    }

    let mut cwd = env::current_dir()?.to_string_lossy().to_string();
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--cwd" {
            cwd = args
                .get(index + 1)
                .ok_or("missing value for `--cwd`")?
                .clone();
            index += 2;
        } else {
            break;
        }
    }
    let task = args[index..].join(" ");
    if task.trim().is_empty() {
        return Err("usage: crust spaces spawn <id> [--cwd <path>] <task>".into());
    }
    Ok((id.to_string(), cwd, task))
}

fn cli_spaces_spawn(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (id, cwd, task) = parse_space_spawn_args(args)?;
    let mut registry = load_spaces_registry().map_err(std::io::Error::other)?;
    let now = chrono::Utc::now().to_rfc3339();
    let task_id = Uuid::new_v4().to_string();
    let space = match registry.find(&id).cloned() {
        Some(mut space) => {
            space.cwd = cwd;
            space.status = SpaceStatus::Running;
            space.task_id = Some(task_id);
            space.task = Some(task.clone());
            space.updated_at = now.clone();
            space
        }
        None => CrustSpace {
            id: id.clone(),
            name: id.clone(),
            session_id: Uuid::new_v4().to_string(),
            cwd,
            status: SpaceStatus::Running,
            process_id: None,
            task_id: Some(task_id),
            task: Some(task.clone()),
            created_at: now.clone(),
            updated_at: now,
        },
    };
    let handoff = build_space_context_handoff_message(&space, &task);
    append_agent_protocol_message(&space_inbox_path(&space.id), &handoff, Some(&space.status))?;
    registry.upsert(space.clone());
    save_spaces_registry(&registry)?;
    println!(
        "Spawned delegated task for space `{}`. Context handoff queued at {}",
        space.id,
        space_inbox_path(&space.id).display()
    );
    Ok(())
}

fn cli_spaces_create(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust spaces create <id> [--cwd <path>] [--task <task>]".into());
    };
    if id.starts_with('-') || id.trim().is_empty() {
        return Err("space id cannot be empty or start with `-`".into());
    }

    let mut cwd = env::current_dir()?.to_string_lossy().to_string();
    let mut task: Option<String> = None;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for `{flag}`"))?;
        match flag {
            "--cwd" => cwd = value.clone(),
            "--task" => task = Some(value.clone()),
            other => return Err(format!("unknown flag `{other}` for spaces create").into()),
        }
        index += 2;
    }

    let mut registry = load_spaces_registry().map_err(std::io::Error::other)?;
    if registry.find(id).is_some() {
        return Err(format!("space `{id}` already exists").into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    let space = CrustSpace {
        id: id.to_string(),
        name: id.to_string(),
        session_id: Uuid::new_v4().to_string(),
        cwd,
        status: SpaceStatus::Idle,
        process_id: None,
        task_id: task.as_ref().map(|_| Uuid::new_v4().to_string()),
        task,
        created_at: now.clone(),
        updated_at: now,
    };
    registry.upsert(space.clone());
    save_spaces_registry(&registry)?;
    println!(
        "Created Crust space `{}` with session {} in {}",
        space.id, space.session_id, space.cwd
    );
    Ok(())
}

fn cli_spaces_attach(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust spaces attach <id>".into());
    };
    let registry = load_spaces_registry().map_err(std::io::Error::other)?;
    let space = registry
        .find(id)
        .ok_or_else(|| format!("space `{id}` not found"))?;
    println!(
        "Space `{}`: status={} cwd={} session={} task={}\ninbox: {}\noutbox: {}",
        space.id,
        space.status,
        space.cwd,
        space.session_id,
        space.task.as_deref().unwrap_or("none"),
        space_inbox_path(&space.id).display(),
        space_outbox_path(&space.id).display()
    );
    Ok(())
}

fn cli_spaces_stop(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust spaces stop <id>".into());
    };
    let mut registry = load_spaces_registry().map_err(std::io::Error::other)?;
    let space = registry
        .find_mut(id)
        .ok_or_else(|| format!("space `{id}` not found"))?;
    space.status = SpaceStatus::Stopped;
    space.updated_at = chrono::Utc::now().to_rfc3339();
    save_spaces_registry(&registry)?;
    println!("Stopped Crust space `{id}`");
    Ok(())
}

fn run_skills_cli(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let skills = load_markdown_skills().map_err(std::io::Error::other)?;
            println!("{}", format_markdown_skills(&skills));
            Ok(())
        }
        Some("show") => {
            let Some(name) = args.get(1) else {
                return Err("usage: crust skills show <name>".into());
            };
            let skill = load_markdown_skill(name).map_err(std::io::Error::other)?;
            println!("{}", format_markdown_skill_detail(&skill));
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown skills command `{command}`. Use `crust skills list` or `crust skills show <name>`."
        )
        .into()),
    }
}

fn run_skill_cli(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(name) = args.first() else {
        return Err("usage: crust skill <name> [args]".into());
    };
    let skill = load_markdown_skill(name).map_err(std::io::Error::other)?;
    if !skill.user_invocable {
        return Err(format!("skill `{}` is not user-invocable", skill.name).into());
    }
    let rendered = render_skill_prompt(&skill, &args[1..].join(" "));
    println!("{rendered}");
    Ok(())
}

async fn run_langgraph_cli(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("add") => cli_langgraph_add(&args[1..]),
        Some("list") => {
            let registry = load_langgraph_registry().map_err(std::io::Error::other)?;
            println!("{}", format_langgraph_registry(&registry));
            Ok(())
        }
        Some("remove") => cli_langgraph_remove(&args[1..]),
        Some("ping") => cli_langgraph_ping(&args[1..]).await,
        Some("help" | "--help" | "-h") | None => {
            print_cli_help();
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown langgraph command `{command}`. Run `crust help` for available commands."
        )
        .into()),
    }
}

fn cli_langgraph_add(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust langgraph add <id> --url <base_url>".into());
    };
    if id.starts_with('-') || id.trim().is_empty() {
        return Err("LangGraph server id cannot be empty or start with `-`".into());
    }

    let mut name: Option<String> = None;
    let mut base_url: Option<String> = None;
    let mut assistant_id: Option<String> = None;
    let mut default_graph: Option<String> = None;
    let mut auth_env: Option<String> = None;
    let mut auth_header: Option<String> = None;
    let mut auth_scheme: Option<String> = None;
    let mut timeout_secs: Option<u64> = None;

    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for `{flag}`"))?;
        match flag {
            "--name" => name = Some(value.clone()),
            "--url" => base_url = Some(normalize_base_url(value)),
            "--assistant-id" => assistant_id = Some(value.clone()),
            "--default-graph" => default_graph = Some(value.clone()),
            "--auth-env" => auth_env = Some(value.clone()),
            "--auth-header" => auth_header = Some(value.clone()),
            "--auth-scheme" => auth_scheme = Some(value.clone()),
            "--timeout-secs" => {
                timeout_secs = Some(
                    value
                        .parse::<u64>()
                        .map_err(|err| format!("invalid --timeout-secs value `{value}`: {err}"))?,
                )
            }
            other => return Err(format!("unknown flag `{other}` for langgraph add").into()),
        }
        index += 2;
    }

    let base_url = base_url.ok_or("missing required `--url <base_url>`")?;
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err("LangGraph --url must start with http:// or https://".into());
    }

    let server = LangGraphServer {
        id: id.to_string(),
        name: name.unwrap_or_else(|| id.to_string()),
        base_url,
        assistant_id,
        default_graph,
        auth_env,
        auth_header,
        auth_scheme,
        timeout_secs,
    };

    let mut registry = load_langgraph_registry().map_err(std::io::Error::other)?;
    registry.upsert(server.clone());
    save_langgraph_registry(&registry)?;
    println!(
        "Registered LangGraph server `{}` at {} in {}",
        server.id,
        server.base_url,
        langgraph_registry_path().display()
    );
    Ok(())
}

fn cli_langgraph_remove(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust langgraph remove <id>".into());
    };
    let mut registry = load_langgraph_registry().map_err(std::io::Error::other)?;
    if !registry.remove(id) {
        return Err(format!("LangGraph server `{id}` not found").into());
    }
    save_langgraph_registry(&registry)?;
    println!("Removed LangGraph server `{id}`");
    Ok(())
}

async fn cli_langgraph_ping(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(id) = args.first().map(String::as_str) else {
        return Err("usage: crust langgraph ping <id>".into());
    };
    let registry = load_langgraph_registry().map_err(std::io::Error::other)?;
    let server = registry
        .find(id)
        .ok_or_else(|| format!("LangGraph server `{id}` not found"))?;
    println!(
        "{}",
        ping_langgraph_server(server)
            .await
            .map_err(|err| std::io::Error::other(err.to_string()))?
    );
    Ok(())
}
