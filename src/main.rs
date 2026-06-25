mod app;
mod cli;
mod tui;

use app::*;
use cli::run_cli;
use crust_core::{
    ContextBuilder, agent_main_run, commands::*, context, env_duration_secs_or_default,
    format_current_session_title, format_scoped_agents, format_token_count, langgraph::*,
    models_generated, parse_scoped_agent_command, scoped_agent_run, skills::*, spaces::*,
};
use crust_types::{
    AgentEvent, AgentState, CrustSpace, LangGraphServer, ScopedAgentStatus, SpaceStatus,
    TaggedAgentEvent,
};
use openrouter_rs::{api::chat::Message, types::Role};
use std::{
    env,
    error::Error,
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio::time;
use tui::ui;
use uuid::Uuid;

use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            MouseEventKind,
        },
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Direction, Layout},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    crust_types::load_env();

    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        return run_cli(args).await;
    }

    run_tui().await
}

async fn run_tui() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // create app and run it
    let mut app = App::new();
    let _ = run_app(&mut terminal, &mut app).await?;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<bool, Box<dyn Error>>
where
    B::Error: Error + 'static,
{
    let (agent_tx, mut agent_rx) = mpsc::channel::<TaggedAgentEvent>(100);

    loop {
        while let Ok(tagged) = agent_rx.try_recv() {
            // Filter out stale events from a previous session
            if tagged.session_id == app.current_session_id {
                app.handle_agent_event(tagged.event);
            }
        }

        if app.last_cursor_toggle.elapsed() >= Duration::from_millis(500) {
            app.cursor_visible = !app.cursor_visible;
            app.last_cursor_toggle = Instant::now();
        }

        terminal.draw(|f| ui(f, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let event = event::read()?;

        match event {
            Event::Mouse(mouse_event) => {
                let area = terminal.size()?;
                let total = area.width;
                let left_divider_x = app.sidebar_width;
                let right_divider_x = total.saturating_sub(app.right_pane_width);

                let h_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(app.sidebar_width),
                        Constraint::Min(10),
                        Constraint::Length(app.right_pane_width),
                    ])
                    .split(area.into());

                let v_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(5)])
                    .split(h_chunks[1]);

                let events_area = v_chunks[0];
                let over_events = mouse_event.column >= events_area.x
                    && mouse_event.column < events_area.x.saturating_add(events_area.width)
                    && mouse_event.row >= events_area.y
                    && mouse_event.row < events_area.y.saturating_add(events_area.height);

                app.hover_divider = mouse_event.column.abs_diff(left_divider_x) <= 1
                    && mouse_event.row < area.height;
                app.hover_right_divider = mouse_event.column.abs_diff(right_divider_x) <= 1
                    && mouse_event.row < area.height;

                match mouse_event.kind {
                    MouseEventKind::ScrollUp if over_events => {
                        app.event_scroll = app.event_scroll.saturating_sub(3);
                        app.follow_mode = false;
                    }
                    MouseEventKind::ScrollDown if over_events => {
                        app.event_scroll = app.event_scroll.saturating_add(3);
                        app.follow_mode = app.event_scroll >= app.event_max_scroll;
                    }
                    MouseEventKind::Down(_) if app.hover_divider => {
                        app.is_dragging_divider = true;
                    }
                    MouseEventKind::Down(_) if app.hover_right_divider => {
                        app.is_dragging_right_divider = true;
                    }
                    MouseEventKind::Drag(_) if app.is_dragging_divider => {
                        app.sidebar_width = mouse_event
                            .column
                            .clamp(10, total.saturating_sub(app.right_pane_width + 20));
                    }
                    MouseEventKind::Drag(_) if app.is_dragging_right_divider => {
                        let max_right = total.saturating_sub(app.sidebar_width + 20).max(16);
                        app.right_pane_width = total
                            .saturating_sub(mouse_event.column)
                            .clamp(16, max_right);
                    }
                    MouseEventKind::Up(_) => {
                        app.is_dragging_divider = false;
                        app.is_dragging_right_divider = false;
                    }
                    _ => {}
                }
                continue;
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global keys that work regardless of focus
                match key.code {
                    KeyCode::Esc => {
                        if let Some(task) = app.agent_task.take() {
                            task.abort();
                            app.handle_agent_event(AgentEvent::Error {
                                message: "Agent run cancelled by user.".to_string(),
                            });
                        } else {
                            break;
                        }
                        continue;
                    }
                    KeyCode::Tab => {
                        app.focus = match app.focus {
                            Focus::Input => Focus::Sidebar,
                            Focus::Sidebar => Focus::Input,
                        };
                        continue;
                    }
                    KeyCode::Char(c) if app.focus == Focus::Input => {
                        app.inputbuffer.insert(app.cursor_pos, c);
                        app.cursor_pos += 1;
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        // Auto-switch sidebar to slash-command mode when input starts with '/'
                        if app.inputbuffer.starts_with('/') {
                            app.sidebar_mode = SidebarMode::SlashCommands;
                            app.slash_command_selected = 0;
                            app.sidebar_scroll = 0;
                        } else if app.sidebar_mode == SidebarMode::SlashCommands {
                            // User typed something else after '/', exit slash-command mode
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.slash_command_selected = 0;
                        }
                        app.retain_input_history_draft();
                        continue;
                    }
                    KeyCode::Backspace if app.focus == Focus::Input => {
                        if app.cursor_pos > 0 {
                            app.cursor_pos -= 1;
                            app.inputbuffer.remove(app.cursor_pos);
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        // If buffer is now empty or doesn't start with '/', exit slash-command mode
                        if app.inputbuffer.is_empty() || !app.inputbuffer.starts_with('/') {
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.slash_command_selected = 0;
                        } else if app.sidebar_mode == SidebarMode::SlashCommands {
                            // Reset selection to top when filter changes
                            app.slash_command_selected = 0;
                            app.sidebar_scroll = 0;
                        }
                        app.retain_input_history_draft();
                        continue;
                    }
                    KeyCode::Left if app.focus == Focus::Input => {
                        if app.cursor_pos > 0 {
                            app.cursor_pos -= 1;
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Right if app.focus == Focus::Input => {
                        if app.cursor_pos < app.inputbuffer.len() {
                            app.cursor_pos += 1;
                        }
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        continue;
                    }
                    KeyCode::Enter if app.focus == Focus::Input => {
                        // If in slash-command mode with a selection, use that command
                        let prompt = if app.sidebar_mode == SidebarMode::SlashCommands {
                            let filtered = get_filtered_commands(&app.inputbuffer);
                            if let Some((cmd, _)) = filtered.get(app.slash_command_selected) {
                                app.inputbuffer.clear();
                                cmd.to_string()
                            } else {
                                app.inputbuffer.trim().to_string()
                            }
                        } else {
                            app.inputbuffer.trim().to_string()
                        };
                        app.inputbuffer.clear();
                        app.cursor_pos = 0;
                        app.cursor_visible = true;
                        app.last_cursor_toggle = Instant::now();
                        app.reset_input_history_navigation();
                        // Exit slash-command mode after submit
                        app.sidebar_mode = SidebarMode::Sessions;
                        app.slash_command_selected = 0;

                        if prompt.is_empty() {
                            continue;
                        }
                        if handle_session_command(app, &prompt, &agent_tx).await {
                            if app.inputstate == InputState::Exit {
                                break;
                            }
                            continue;
                        }

                        if app.agent_task.is_some() {
                            app.handle_agent_event(AgentEvent::Error {
                                message: "Agent is already running.".to_string(),
                            });
                            continue;
                        }

                        app.append_input_history(&prompt);

                        // Capture current session ID before spawning task
                        let current_session_id = app.current_session_id.clone();

                        app.handle_agent_event(AgentEvent::UserSubmitted {
                            prompt: prompt.clone(),
                        });
                        terminal.draw(|f| ui(f, app))?;

                        let sessionmanager = Arc::clone(&app.sessionmanager);
                        let tx = agent_tx.clone();
                        let spawn_session_id = current_session_id.clone();
                        app.agent_task = Some(tokio::spawn(async move {
                            if let Err(err) = agent_main_run(
                                sessionmanager,
                                prompt,
                                current_session_id,
                                tx.clone(),
                            )
                            .await
                            {
                                let _ = tx
                                    .send(TaggedAgentEvent {
                                        session_id: spawn_session_id.clone(),
                                        event: AgentEvent::Error {
                                            message: err.to_string(),
                                        },
                                    })
                                    .await;
                            }
                            let _ = tx
                                .send(TaggedAgentEvent {
                                    session_id: spawn_session_id,
                                    event: AgentEvent::Finished,
                                })
                                .await;
                        }));
                        continue;
                    }
                    _ => {}
                }

                // Focus-specific key handling
                match app.focus {
                    Focus::Input => match key.code {
                        KeyCode::Up => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected =
                                    app.slash_command_selected.saturating_sub(1);
                                app.sidebar_scroll = app.sidebar_scroll.saturating_sub(1);
                            } else {
                                app.load_previous_input_history();
                            }
                        }
                        KeyCode::Down => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                let max = filtered.len().saturating_sub(1);
                                app.slash_command_selected =
                                    (app.slash_command_selected + 1).min(max);
                                app.sidebar_scroll = (app.sidebar_scroll + 1).min(max as u16);
                            } else {
                                app.load_next_input_history();
                            }
                        }
                        KeyCode::PageUp => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected =
                                    app.slash_command_selected.saturating_sub(5);
                                app.sidebar_scroll = app.sidebar_scroll.saturating_sub(5);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_sub(10);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::PageDown => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                let max = filtered.len().saturating_sub(1);
                                app.slash_command_selected =
                                    (app.slash_command_selected + 5).min(max);
                                app.sidebar_scroll = (app.sidebar_scroll + 5).min(max as u16);
                            } else {
                                app.event_scroll = app.event_scroll.saturating_add(10);
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::Home => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                app.slash_command_selected = 0;
                                app.sidebar_scroll = 0;
                            } else {
                                app.event_scroll = 0;
                                app.follow_mode = false;
                            }
                        }
                        KeyCode::End => {
                            if app.sidebar_mode == SidebarMode::SlashCommands {
                                let filtered = get_filtered_commands(&app.inputbuffer);
                                app.slash_command_selected = filtered.len().saturating_sub(1);
                                app.sidebar_scroll =
                                    filtered.len().saturating_sub(1).min(u16::MAX as usize) as u16;
                            } else {
                                app.event_scroll = usize::MAX;
                                app.follow_mode = true;
                            }
                        }
                        _ => {}
                    },
                    Focus::Sidebar => match key.code {
                        KeyCode::Left => {
                            app.sidebar_mode = SidebarMode::Sessions;
                            app.sidebar_scroll = 0;
                            app.sidebar_selected = 0;
                        }
                        KeyCode::Right => {
                            app.sidebar_mode = SidebarMode::Models;
                            app.sidebar_scroll = 0;
                            app.sidebar_selected = 0;
                        }
                        KeyCode::Up => {
                            app.sidebar_scroll = app.sidebar_scroll.saturating_sub(1);
                            app.sidebar_selected = app.sidebar_selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            if app.sidebar_mode == SidebarMode::Models {
                                let model_count = models_generated::OPENROUTER_MODELS.len();
                                app.sidebar_selected =
                                    (app.sidebar_selected + 1).min(model_count.saturating_sub(1));
                                app.sidebar_scroll = (app.sidebar_scroll + 1)
                                    .min(model_count.saturating_sub(1) as u16);
                            }
                        }
                        KeyCode::Enter => {
                            if app.sidebar_mode == SidebarMode::Models {
                                let selected_model =
                                    models_generated::OPENROUTER_MODELS.get(app.sidebar_selected);
                                if let Some(model) = selected_model {
                                    let mut sm = app.sessionmanager.lock().await;
                                    if let Some(session) = sm.get_current_session_mut() {
                                        session.config.modelname = model.id.to_string();
                                        app.current_session_title =
                                            format_current_session_title(session);
                                    }
                                    app.eventlog.push(LogEntry::System(format!(
                                        "Switched model to: {}",
                                        model.id
                                    )));
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

async fn handle_session_command(
    app: &mut App,
    prompt: &str,
    agent_tx: &mpsc::Sender<TaggedAgentEvent>,
) -> bool {
    if prompt == "/exit" {
        app.inputstate = InputState::Exit;
        app.eventlog
            .push(LogEntry::System("Crust agent quitting.....".to_string()));
        return true;
    }

    if prompt == "/new" || prompt.starts_with("/new ") {
        let session_name = prompt.strip_prefix("/new").unwrap_or("").trim().to_string();
        if session_name.is_empty() {
            app.eventlog
                .push(LogEntry::Error("Usage: /new <session_name>".to_string()));
            return true;
        }

        let mut sessionmanager = app.sessionmanager.lock().await;
        if sessionmanager.session_name_exists(&session_name) {
            app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' already exists"
            )));
            return true;
        }
        let new_session = sessionmanager.create_session_with_core(
            session_name,
            &app.system_message,
            app.core_kind,
        );
        app.current_session_title = format_current_session_title(new_session);
        app.current_session_id = new_session.id.clone();
        app.eventlog.push(LogEntry::System(format!(
            "Created and switched to new session: {} ({})",
            new_session.name, new_session.id
        )));
        drop(sessionmanager);
        app.sync_input_history_from_current_session().await;
        return true;
    }

    if prompt == "/clear" {
        let mut sessionmanager = app.sessionmanager.lock().await;
        sessionmanager.clear_current_session();
        if let Some(session) = sessionmanager.get_current_session() {
            app.current_session_title = format_current_session_title(session);
        }
        app.eventlog.clear();
        app.eventlog.push(LogEntry::System(
            "Cleared current session history.".to_string(),
        ));
        app.event_scroll = 0;
        app.follow_mode = true;
        drop(sessionmanager);
        app.sync_input_history_from_current_session().await;
        return true;
    }

    if prompt == "/context" {
        let context_builder = ContextBuilder::default();
        let sessionmanager = app.sessionmanager.lock().await;
        if let Some(session) = sessionmanager.get_current_session() {
            let context = context_builder.build_context(session, &session.config);
            let estimated_tokens = context_builder.estimate_context_tokens(&context);
            let context_window = context_builder.context_window_tokens(&session.config);
            let estimated_ratio =
                context_builder.estimated_context_ratio(&context, &session.config);
            let last_api_ratio = context_builder
                .context_ratio_from_api_usage(session.latest_prompt_tokens, &session.config);
            app.eventlog.push(LogEntry::System(format!(
                "Context status:\nmodel: {}\ncontext window: {} toks\nbuilt context: {} messages, est. {} toks ({:.1}%)\nlast OpenRouter prompt usage: {} toks ({:.1}%)\nsummary: {}\ncompacted_until: {} / {} messages",
                session.config.modelname,
                format_token_count(context_window),
                context.len(),
                format_token_count(estimated_tokens),
                estimated_ratio * 100.0,
                format_token_count(session.latest_prompt_tokens as usize),
                last_api_ratio * 100.0,
                if session.summary.is_some() { "present" } else { "none" },
                session.compacted_until,
                session.messages.len(),
            )));
        }
        return true;
    }

    if prompt == "/compact" {
        let context_builder = ContextBuilder::default();
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.compact_current_session_to_recent(context_builder.min_recent_messages)
        {
            Ok(cutoff) => {
                if let Some(session) = sessionmanager.get_current_session() {
                    app.current_session_title = format_current_session_title(session);
                }
                app.eventlog.push(LogEntry::System(format!(
                    "Compacted session history through message index {cutoff}. The model context will now use the summary plus the recent tail; semantic memory remains ephemeral per request."
                )));
            }
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Compact failed: {err}"))),
        }
        return true;
    }

    if prompt == "/agents" {
        app.eventlog
            .push(LogEntry::System(format_scoped_agents(&app.scoped_agents)));
        return true;
    }

    if let Some(name_or_id) = prompt.strip_prefix("/agent-cancel ") {
        let name_or_id = name_or_id.trim();
        match app.find_scoped_agent_index(name_or_id) {
            Some(index) => {
                let id = app.scoped_agents[index].id.clone();
                if let Some(task) = app.scoped_agent_tasks.remove(&id) {
                    task.abort();
                }
                app.scoped_agents[index].status = ScopedAgentStatus::Cancelled;
                app.push_scoped_agent_event(&id, "cancelled by user".to_string());
                app.eventlog.push(LogEntry::System(format!(
                    "Cancelled scoped agent `{name_or_id}`."
                )));
            }
            None => app.eventlog.push(LogEntry::Error(format!(
                "Scoped agent `{name_or_id}` not found."
            ))),
        }
        return true;
    }

    if let Some(name_or_id) = prompt.strip_prefix("/agent-result ") {
        let name_or_id = name_or_id.trim();
        match app.find_scoped_agent_index(name_or_id) {
            Some(index) => {
                let agent = &app.scoped_agents[index];
                let result = agent.result.as_deref().unwrap_or("No result yet.");
                app.eventlog.push(LogEntry::System(format!(
                    "Scoped agent `{}` result:\n{}",
                    agent.name,
                    context::truncate_middle(result, 10_000)
                )));
            }
            None => app.eventlog.push(LogEntry::Error(format!(
                "Scoped agent `{name_or_id}` not found."
            ))),
        }
        return true;
    }

    if prompt == "/agent" || prompt.starts_with("/agent ") {
        let Some((name, task)) = parse_scoped_agent_command(prompt) else {
            app.eventlog.push(LogEntry::Error(
                "Usage: /agent <name> <task>. Scoped agents are explicit only; they are never auto-routed."
                    .to_string(),
            ));
            return true;
        };

        if app.find_scoped_agent_index(&name).is_some() {
            app.eventlog.push(LogEntry::Error(format!(
                "Scoped agent `{name}` already exists. Choose a unique name."
            )));
            return true;
        }

        let id = Uuid::new_v4().to_string();
        let max_steps = env::var("CRUST_SCOPED_AGENT_MAX_STEPS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(100);
        let scoped_timeout = env_duration_secs_or_default("CRUST_SCOPED_AGENT_TIMEOUT_SECS", 180);
        let sessionmanager = Arc::clone(&app.sessionmanager);
        let tx = agent_tx.clone();
        let session_id = app.current_session_id.clone();

        app.handle_agent_event(AgentEvent::ScopedAgentStarted {
            id: id.clone(),
            name: name.clone(),
            task: task.clone(),
            max_steps,
        });

        let run_id = id.clone();
        let handle = tokio::spawn(async move {
            let result = time::timeout(
                scoped_timeout,
                scoped_agent_run(
                    sessionmanager,
                    task,
                    run_id.clone(),
                    session_id.clone(),
                    max_steps,
                    tx.clone(),
                ),
            )
            .await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    let _ = tx
                        .send(TaggedAgentEvent {
                            session_id,
                            event: AgentEvent::ScopedAgentError {
                                id: run_id,
                                message: err.to_string(),
                            },
                        })
                        .await;
                }
                Err(_) => {
                    let _ = tx
                        .send(TaggedAgentEvent {
                            session_id,
                            event: AgentEvent::ScopedAgentError {
                                id: run_id,
                                message: format!(
                                    "timed out after {} seconds",
                                    scoped_timeout.as_secs()
                                ),
                            },
                        })
                        .await;
                }
            }
        });
        app.scoped_agent_tasks.insert(id, handle);
        return true;
    }

    if prompt == "/skills" {
        match load_markdown_skills() {
            Ok(skills) => app
                .eventlog
                .push(LogEntry::System(format_markdown_skills(&skills))),
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Markdown skills failed: {err}"))),
        }
        return true;
    }

    if prompt == "/skill" || prompt.starts_with("/skill ") {
        let Some((name, skill_args)) = parse_skill_command(prompt) else {
            app.eventlog
                .push(LogEntry::Error("Usage: /skill <name> [args]".to_string()));
            return true;
        };
        if app.agent_task.is_some() {
            app.eventlog
                .push(LogEntry::Error("Agent is already running.".to_string()));
            return true;
        }
        match load_markdown_skill(name) {
            Ok(skill) => {
                if !skill.user_invocable {
                    app.eventlog.push(LogEntry::Error(format!(
                        "Skill `{}` is not user-invocable.",
                        skill.name
                    )));
                    return true;
                }
                let rendered_prompt = render_skill_prompt(&skill, skill_args);
                app.handle_agent_event(AgentEvent::UserSubmitted {
                    prompt: rendered_prompt.clone(),
                });
                let sessionmanager = Arc::clone(&app.sessionmanager);
                let current_session_id = app.current_session_id.clone();
                let tx = agent_tx.clone();
                let spawn_session_id = current_session_id.clone();
                app.agent_task = Some(tokio::spawn(async move {
                    if let Err(err) = agent_main_run(
                        sessionmanager,
                        rendered_prompt,
                        current_session_id,
                        tx.clone(),
                    )
                    .await
                    {
                        let _ = tx
                            .send(TaggedAgentEvent {
                                session_id: spawn_session_id,
                                event: AgentEvent::Error {
                                    message: format!("Skill run failed: {err}"),
                                },
                            })
                            .await;
                    }
                }));
            }
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Skill failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph" || prompt == "/langgraph list" {
        match load_langgraph_registry() {
            Ok(registry) => app
                .eventlog
                .push(LogEntry::System(format_langgraph_registry(&registry))),
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph registry failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph add" || prompt.starts_with("/langgraph add ") {
        let Some((id, url)) = parse_langgraph_add_command(prompt) else {
            app.eventlog.push(LogEntry::Error(
                "Usage: /langgraph add <id> <http-or-https-url>".to_string(),
            ));
            return true;
        };
        let base_url = normalize_base_url(url);
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            app.eventlog.push(LogEntry::Error(
                "LangGraph URL must start with http:// or https://".to_string(),
            ));
            return true;
        }

        match load_langgraph_registry() {
            Ok(mut registry) => {
                let server = LangGraphServer {
                    id: id.to_string(),
                    name: id.to_string(),
                    base_url,
                    assistant_id: None,
                    default_graph: None,
                    auth_env: None,
                    auth_header: None,
                    auth_scheme: None,
                    timeout_secs: None,
                };
                registry.upsert(server.clone());
                match save_langgraph_registry(&registry) {
                    Ok(()) => app.eventlog.push(LogEntry::System(format!(
                        "Registered LangGraph server `{}` at {}",
                        server.id, server.base_url
                    ))),
                    Err(err) => app.eventlog.push(LogEntry::Error(format!(
                        "LangGraph registry save failed: {err}"
                    ))),
                }
            }
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph registry failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph run" || prompt.starts_with("/langgraph run ") {
        let Some(command) = parse_langgraph_run_command(prompt) else {
            app.eventlog.push(LogEntry::Error(
                "Usage: /langgraph run <server-id> [--context-scope <scope>] [--file <path>] [--skill <name>] <input>".to_string(),
            ));
            return true;
        };

        match load_langgraph_registry() {
            Ok(registry) => match registry.find(&command.server_id).cloned() {
                Some(server) => {
                    let session = app
                        .sessionmanager
                        .try_lock()
                        .ok()
                        .and_then(|sm| sm.get_current_session().cloned());
                    let handoff = build_langgraph_handoff_payload(
                        &command.input,
                        session.as_ref(),
                        command.context_scope.as_deref(),
                        &command.files,
                        &command.skill_names,
                    );
                    match handoff {
                        Ok(handoff) => {
                            match run_langgraph_registered(&server, &command.input, Some(handoff))
                                .await
                            {
                                Ok(record) => app.eventlog.push(LogEntry::System(format!(
                                    "LangGraph run `{}` completed:\n{}",
                                    record.id,
                                    context::truncate_middle(
                                        record
                                            .final_text
                                            .as_deref()
                                            .unwrap_or("No final text recorded."),
                                        20_000
                                    )
                                ))),
                                Err(err) => app
                                    .eventlog
                                    .push(LogEntry::Error(format!("LangGraph run failed: {err}"))),
                            }
                        }
                        Err(err) => app
                            .eventlog
                            .push(LogEntry::Error(format!("LangGraph handoff failed: {err}"))),
                    }
                }
                None => app.eventlog.push(LogEntry::Error(format!(
                    "LangGraph server `{}` not found. Run /langgraph list.",
                    command.server_id
                ))),
            },
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph registry failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph runs" {
        match list_langgraph_run_records() {
            Ok(records) => app
                .eventlog
                .push(LogEntry::System(format_langgraph_runs(&records))),
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph runs failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph result" || prompt.starts_with("/langgraph result ") {
        let Some((run_id, inject)) = parse_langgraph_result_command(prompt) else {
            app.eventlog.push(LogEntry::Error(
                "Usage: /langgraph result <run_id> [--inject]".to_string(),
            ));
            return true;
        };
        match load_langgraph_run_record(&run_id) {
            Ok(record) => {
                if inject {
                    let summary = langgraph_result_injection_summary(&record);
                    {
                        let mut sm = app.sessionmanager.lock().await;
                        sm.add_message_to_current(Message::new(Role::Assistant, summary.clone()));
                    }
                    app.eventlog.push(LogEntry::System(format!(
                        "Injected compact LangGraph result summary into the current session.\n\n{}",
                        context::truncate_middle(&summary, 8_000)
                    )));
                } else {
                    app.eventlog
                        .push(LogEntry::System(format_langgraph_run_result(&record)));
                }
            }
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph result failed: {err}"))),
        }
        return true;
    }

    if prompt == "/langgraph cancel" || prompt.starts_with("/langgraph cancel ") {
        let Some(run_id) = parse_langgraph_single_arg_command(prompt, "/langgraph cancel") else {
            app.eventlog.push(LogEntry::Error(
                "Usage: /langgraph cancel <run_id>".to_string(),
            ));
            return true;
        };
        match load_langgraph_run_record(run_id) {
            Ok(record) => match cancel_langgraph_run(&record).await {
                Ok(message) => app.eventlog.push(LogEntry::System(message)),
                Err(err) => app
                    .eventlog
                    .push(LogEntry::Error(format!("LangGraph cancel failed: {err}"))),
            },
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("LangGraph cancel failed: {err}"))),
        }
        return true;
    }

    if prompt == "/spaces" {
        match load_spaces_registry() {
            Ok(registry) => app
                .eventlog
                .push(LogEntry::System(format_spaces_registry(&registry))),
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Spaces registry failed: {err}"))),
        }
        return true;
    }

    if prompt == "/space-create" || prompt.starts_with("/space-create ") {
        let id = prompt.strip_prefix("/space-create").unwrap_or("").trim();
        if id.is_empty() {
            app.eventlog
                .push(LogEntry::Error("Usage: /space-create <id>".to_string()));
            return true;
        }
        let mut registry = match load_spaces_registry() {
            Ok(registry) => registry,
            Err(err) => {
                app.eventlog
                    .push(LogEntry::Error(format!("Spaces registry failed: {err}")));
                return true;
            }
        };
        if registry.find(id).is_some() {
            app.eventlog
                .push(LogEntry::Error(format!("Space `{id}` already exists.")));
            return true;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let cwd = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        let space = CrustSpace {
            id: id.to_string(),
            name: id.to_string(),
            session_id: Uuid::new_v4().to_string(),
            cwd,
            status: SpaceStatus::Idle,
            process_id: None,
            task_id: None,
            task: None,
            created_at: now.clone(),
            updated_at: now,
        };
        registry.upsert(space.clone());
        match save_spaces_registry(&registry) {
            Ok(()) => app.eventlog.push(LogEntry::System(format!(
                "Created space `{}` with session {}",
                space.id, space.session_id
            ))),
            Err(err) => app.eventlog.push(LogEntry::Error(format!(
                "Spaces registry save failed: {err}"
            ))),
        }
        return true;
    }

    if prompt == "/space-attach" || prompt.starts_with("/space-attach ") {
        let id = prompt.strip_prefix("/space-attach").unwrap_or("").trim();
        if id.is_empty() {
            app.eventlog
                .push(LogEntry::Error("Usage: /space-attach <id>".to_string()));
            return true;
        }
        match load_spaces_registry() {
            Ok(registry) => match registry.find(id) {
                Some(space) => app.eventlog.push(LogEntry::System(format!(
                    "Space `{}`: status={} cwd={} session={} task={}\ninbox: {}\noutbox: {}",
                    space.id,
                    space.status,
                    space.cwd,
                    space.session_id,
                    space.task.as_deref().unwrap_or("none"),
                    space_inbox_path(&space.id).display(),
                    space_outbox_path(&space.id).display()
                ))),
                None => app
                    .eventlog
                    .push(LogEntry::Error(format!("Space `{id}` not found."))),
            },
            Err(err) => app
                .eventlog
                .push(LogEntry::Error(format!("Spaces registry failed: {err}"))),
        }
        return true;
    }

    if prompt == "/space-stop" || prompt.starts_with("/space-stop ") {
        let id = prompt.strip_prefix("/space-stop").unwrap_or("").trim();
        if id.is_empty() {
            app.eventlog
                .push(LogEntry::Error("Usage: /space-stop <id>".to_string()));
            return true;
        }
        let mut registry = match load_spaces_registry() {
            Ok(registry) => registry,
            Err(err) => {
                app.eventlog
                    .push(LogEntry::Error(format!("Spaces registry failed: {err}")));
                return true;
            }
        };
        match registry.find_mut(id) {
            Some(space) => {
                space.status = SpaceStatus::Stopped;
                space.updated_at = chrono::Utc::now().to_rfc3339();
                match save_spaces_registry(&registry) {
                    Ok(()) => app
                        .eventlog
                        .push(LogEntry::System(format!("Stopped space `{id}`."))),
                    Err(err) => app.eventlog.push(LogEntry::Error(format!(
                        "Spaces registry save failed: {err}"
                    ))),
                }
            }
            None => app
                .eventlog
                .push(LogEntry::Error(format!("Space `{id}` not found."))),
        }
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/delete ") {
        let session_name = session_name.trim();
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => {
                if let Some(current) = sessionmanager.get_current_session() {
                    if current.name == session_name {
                        app.eventlog.push(LogEntry::Error(format!(
                            "Cannot delete the active session '{session_name}'"
                        )));
                        return true;
                    }
                }
                match sessionmanager.delete_session(&session_id) {
                    Ok(_) => app
                        .eventlog
                        .push(LogEntry::System(format!("Deleted session: {session_name}"))),
                    Err(e) => app.eventlog.push(LogEntry::Error(format!("Error: {e}"))),
                }
            }
            None => app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' not found"
            ))),
        }
        return true;
    }

    if let Some(session_name) = prompt.strip_prefix("/switch ") {
        let session_name = session_name.trim();
        // Block session switching while agent is running
        if app.agent_task.is_some() {
            app.eventlog.push(LogEntry::Error(
                "Cannot switch sessions while agent is running. Press Esc to cancel first."
                    .to_string(),
            ));
            return true;
        }
        let mut sessionmanager = app.sessionmanager.lock().await;
        match sessionmanager.find_session_id_by_name(session_name) {
            Some(session_id) => match sessionmanager.switch_session(&session_id) {
                Ok(session) => {
                    app.current_session_id = session.id.clone();
                    app.current_session_title = format_current_session_title(session);
                    // Clear and rebuild event log from the switched session's messages
                    app.eventlog.clear();
                    app.active_assistant_log_index = None;
                    app.active_thinking_log_index = None;
                    for message in &session.messages {
                        if let Some(log_entry) = message_to_log_entry(message) {
                            app.eventlog.push(log_entry);
                        }
                    }
                    app.eventlog.push(LogEntry::System(format!(
                        "Switched to session: {} ({} messages)",
                        session.name,
                        session.messages.len()
                    )));
                    app.event_scroll = 0;
                    app.follow_mode = true;
                    app.agentstate = AgentState::Idle;
                    drop(sessionmanager);
                    app.sync_input_history_from_current_session().await;
                }
                Err(e) => app.eventlog.push(LogEntry::Error(format!("Error: {e}"))),
            },
            None => app.eventlog.push(LogEntry::Error(format!(
                "Error: session named '{session_name}' not found"
            ))),
        }
        return true;
    }

    false
}
