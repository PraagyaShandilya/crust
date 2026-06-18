# crust

`crust` is a terminal coding agent written in Rust. It provides a Ratatui/Crossterm TUI, persists local chat sessions, streams responses from OpenRouter, and gives the model tools for web search, shell commands, reading files, writing files, and simple text replacement edits.

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

Sessions are saved locally in `.crust_sessions/`.

## In-App Commands

- `/new <session_name>` creates and switches to a new session.
- `/switch <session_name>` switches sessions.
- `/delete <session_name>` deletes a session.
- `/clear` clears the current session history.
- `/context` shows context usage.
- `/compact` compacts older session history.
- `/exit` exits the application.
