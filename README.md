# crust

`crust` is a terminal coding agent written in Rust. It provides a Ratatui/Crossterm TUI, persists local chat sessions, streams responses from OpenRouter, and gives the model tools for web search, shell commands, reading files, writing files, simple text replacement edits, and registered LangGraph dev-server handoffs.

It also supports explicit scoped child agents and local Markdown skills.

The project now supports native Windows and Unix-like systems. By default, shell commands run through PowerShell on Windows and Bash on Linux/macOS.

## Requirements

- Rust toolchain with Cargo installed.
- An OpenRouter API key.
- A Tavily API key if you want the web search tool to work.
- A terminal that supports interactive TUI apps.

## Setup

Clone the repository and enter the project directory:

```sh
git clone <repo-url>
cd crust
```

Create a `.env` file in the project root:

```env
OPENROUTER_API_KEY=your_openrouter_key
TAVILY_API_KEY=your_tavily_key
OPENROUTER_MAIN_MODEL=moonshotai/kimi-latest
MAX_AGENT_STEPS=10
CRUST_SHELL=auto
CRUST_SCOPED_AGENT_MAX_STEPS=100
CRUST_SCOPED_AGENT_TIMEOUT_SECS=180
```

Build and run:

```sh
cargo run
```

Run checks and tests:

```sh
cargo check
cargo test
```

## Windows Setup

1. Install Rust from <https://rustup.rs/>. The default MSVC toolchain is recommended.
2. Open PowerShell in the repository directory.
3. Create the `.env` file shown above.
4. Run:

```powershell
cargo run
```

On Windows, `CRUST_SHELL=auto` uses `powershell.exe -NoProfile -Command`. You can override the shell:

```env
CRUST_SHELL=powershell
CRUST_SHELL=cmd
CRUST_SHELL=bash
```

Use `bash` only if Bash is available on your PATH, such as through Git Bash, MSYS2, or WSL interop.

## Linux Setup

1. Install Rust from <https://rustup.rs/> or your distribution package manager.
2. Make sure Bash is available.
3. Create the `.env` file shown above.
4. Run:

```sh
cargo run
```

On Linux, `CRUST_SHELL=auto` uses `bash -lc`. You can set `CRUST_SHELL=bash` explicitly, but `auto` is the usual choice.

## Configuration

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `OPENROUTER_API_KEY` | Yes | none | API key used for model calls. |
| `TAVILY_API_KEY` | For web search | none | API key used by the web search tool. |
| `OPENROUTER_MAIN_MODEL` | No | `moonshotai/kimi-latest` | OpenRouter model id. |
| `MAX_AGENT_STEPS` | No | `10` | Maximum tool/assistant loop iterations per user prompt. |
| `CRUST_SHELL` | No | `auto` | Shell used by `shell_calling_tool`: `auto`, `bash`, `powershell`, or `cmd`. |
| `CRUST_SCOPED_AGENT_MAX_STEPS` | No | `100` | Maximum loop steps for scoped child agents. |
| `CRUST_SCOPED_AGENT_TIMEOUT_SECS` | No | `180` | Total runtime timeout for scoped child agents. |
| `CRUST_SKILLS_DIR` | No | `skills` | Directory containing Markdown skill files. |

Sessions are saved locally in `.crust_sessions/`. LangGraph registry and run records are saved in `.crust_langgraph/`. Spaces registry and IPC files are saved in `.crust_spaces/`.

## In-App Commands

- `/new <session_name>` creates and switches to a new session.
- `/switch <session_name>` switches sessions.
- `/delete <session_name>` deletes a session.
- `/clear` clears the current session history.
- `/context` shows context usage.
- `/compact` compacts older session history.
- `/skills` lists Markdown skills loaded from `skills/` or `CRUST_SKILLS_DIR`.
- `/skill <name> [args]` invokes a Markdown skill explicitly.
- `/langgraph list` lists registered LangGraph dev servers.
- `/langgraph add <id> <url>` registers a LangGraph dev server from the TUI.
- `/langgraph run <id> [--context-scope <scope>] [--file <path>] [--skill <name>] <input>` runs a registered LangGraph workflow explicitly.
- `/langgraph runs` lists persisted LangGraph run records.
- `/langgraph result <run_id> [--inject]` shows or injects a compact LangGraph result summary.
- `/langgraph cancel <run_id>` requests cancellation for a LangGraph run with a provider run id.
- `/spaces` lists Crust spaces.
- `/space-create <id>` creates a Crust space record.
- `/space-attach <id>` shows space details and IPC paths.
- `/space-stop <id>` marks a space stopped.
- `/agent <name> <task>` starts an explicit scoped child agent with a short context.
- `/agents` lists scoped child agents in the current TUI session.
- `/agent-cancel <name-or-id>` cancels a scoped child agent.
- `/agent-result <name-or-id>` shows a scoped child agent result.
- `/exit` exits the application.

Scoped child agents are explicit only. They are not auto-routed, and nested child-agent spawning is blocked. Normal and scoped sessions use default session context only: system prompt, compacted summary, recent messages, and local session history.

## CLI Commands

```sh
crust skills list
crust skills show <name>
crust skill <name> [args]
crust langgraph add <id> --url <base_url> [--assistant-id <id>] [--auth-env <var>]
crust langgraph list
crust langgraph remove <id>
crust langgraph ping <id>
crust spaces list
crust spaces create <id> [--cwd <path>] [--task <task>]
crust spaces spawn <id> [--cwd <path>] <task>
crust spaces attach <id>
crust spaces stop <id>
```

## LangGraph

LangGraph servers are registered locally in `.crust_langgraph/servers.json`. Crust talks to registered HTTP or HTTPS dev servers only; model-facing LangGraph calls cannot use arbitrary URLs.

Runs create LangGraph threads, stream `/threads/{thread_id}/runs/stream`, parse SSE events, and persist compact run records under `.crust_langgraph/runs/` with raw events under `.crust_langgraph/raw_events/`.

## Spaces

Spaces are tmux-inspired Crust work slots tracked in `.crust_spaces/spaces.json`. Each space has an id, session id, cwd, status, process/task metadata, and per-space IPC paths.

`crust spaces spawn <id> <task>` creates or updates a space, marks it running, and writes a validated `AgentProtocol` context handoff message to `.crust_spaces/<id>/ipc/inbox.jsonl`.

The `AgentProtocol` envelope is versioned and typed: `protocol_version`, `message_id`, `space_id`, `task_id`, `message_type`, `correlation_id`, `timestamp`, and `payload`. Invalid versions, missing fields, mismatched payload variants, invalid payloads, and unsupported state transitions are rejected before IPC writes.

The right-side TUI pane lists spaces, LangGraph runs, and scoped agents. It is mouse-resizable like the left sidebar.

## Skills

Markdown skills live in `skills/<name>/SKILL.md` by default. Set `CRUST_SKILLS_DIR` to use another directory.

Example skill file:

```md
---
description: Review Rust changes for correctness and safety
allowed-tools: shell_calling_tool, read_file_tool
user-invocable: true
---

# rust-review

Review Rust changes for correctness, safety, error handling, and missing tests. Arguments: {{args}}
```

## Scoped Agents

Scoped agents are small child runs created inside the TUI session:

```text
/agent path-safety Fix path containment in the file tools and run cargo check
```

They use a short context made from the task, parent summary, and recent parent messages. Their operation is shown in a compact `Scoped Agents` pane. Only the final result is merged back into the parent session history.

Scoped agents currently have a restricted tool set: shell, read file, and edit file. They cannot spawn nested agents.
