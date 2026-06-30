use crate::{App, Focus, InputState, LogEntry, SidebarMode};
use crust_core::{
    commands::get_filtered_commands, compact_tool_call_text, compact_tool_result_text, context,
    langgraph::list_langgraph_run_records, models_generated, spaces::load_spaces_registry,
};
use crust_types::{AgentState, CoreKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(crate) fn ui(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(app.sidebar_width),
            Constraint::Min(10),
            Constraint::Length(app.right_pane_width),
        ])
        .split(area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(h_chunks[1]);

    let (sidebar_title, sidebar_content, sidebar_scroll) = {
        match app.sidebar_mode {
            SidebarMode::SlashCommands => {
                let filtered = get_filtered_commands(&app.inputbuffer);
                let lines: Vec<String> = filtered
                    .iter()
                    .enumerate()
                    .map(|(i, (cmd, _desc))| {
                        let marker = if i == app.slash_command_selected {
                            "> "
                        } else {
                            "  "
                        };
                        format!("{}{}", marker, cmd)
                    })
                    .collect();
                let content = if lines.is_empty() {
                    "No matching commands".to_string()
                } else {
                    lines.join("\n")
                };
                (" Commands ", content, app.sidebar_scroll)
            }
            _ => match app.sessionmanager.try_lock() {
                Ok(sm) => match app.sidebar_mode {
                    SidebarMode::Sessions => {
                        let current_name = sm
                            .get_current_session()
                            .map(|s| s.name.as_str())
                            .unwrap_or("");
                        let names = sm.list_session_names();
                        let lines: Vec<String> = names
                            .iter()
                            .map(|name| {
                                let active = if *name == current_name { " ●" } else { "" };
                                format!("{}{}", name, active)
                            })
                            .collect();
                        let content = if lines.is_empty() {
                            "No sessions".to_string()
                        } else {
                            lines.join("\n")
                        };
                        (" Sessions ", content, 0u16)
                    }
                    SidebarMode::Models => {
                        let current_id = sm
                            .get_current_session()
                            .map(|s| s.config.modelname.as_str())
                            .unwrap_or("");

                        let lines: Vec<String> = models_generated::OPENROUTER_MODELS
                            .iter()
                            .enumerate()
                            .map(|(i, m)| {
                                let marker = if i == app.sidebar_selected {
                                    "> "
                                } else {
                                    "  "
                                };
                                let active = if m.id == current_id { " ●" } else { "" };
                                format!("{}{}{}", marker, m.id, active)
                            })
                            .collect();

                        let content = lines.join("\n");
                        (" Models ", content, app.sidebar_scroll)
                    }
                    SidebarMode::SlashCommands => unreachable!(),
                },
                Err(_) => (" Loading... ", "Loading...".to_string(), 0u16),
            },
        }
    };

    let border_style = if app.is_dragging_divider || app.hover_divider {
        Style::default().fg(Color::Yellow)
    } else if app.focus == Focus::Sidebar || app.sidebar_mode == SidebarMode::SlashCommands {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let sidebar = Paragraph::new(sidebar_content)
        .block(
            Block::default()
                .title(sidebar_title)
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .scroll((sidebar_scroll, 0))
        .wrap(Wrap { trim: false });

    let (events_area, scoped_agents_area): (Rect, Option<Rect>) = if app.scoped_agents.is_empty() {
        (v_chunks[0], None)
    } else {
        let event_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(7)])
            .split(v_chunks[0]);
        (event_chunks[0], Some(event_chunks[1]))
    };

    let events_view_height = usize::from(events_area.height.saturating_sub(2));
    let events_view_width = usize::from(events_area.width.saturating_sub(2)).max(1);

    let rendered_lines: Vec<Line> = app
        .eventlog
        .iter()
        .flat_map(|entry| log_entry_to_wrapped_lines(entry, events_view_width))
        .collect();
    let rendered_height = rendered_lines.len();

    let max_scroll = rendered_height.saturating_sub(events_view_height);

    if app.follow_mode {
        app.event_scroll = max_scroll;
    } else {
        app.event_scroll = app.event_scroll.min(max_scroll);
    }
    app.event_max_scroll = max_scroll;

    let visible_lines: Vec<Line> = rendered_lines
        .into_iter()
        .skip(app.event_scroll)
        .take(events_view_height)
        .collect();

    let event_border_color = match app.agentstate {
        AgentState::Thinking | AgentState::Tool => Color::Yellow,
        AgentState::Done => Color::Green,
        AgentState::Error => Color::Red,
        AgentState::Idle => Color::Blue,
    };

    let events_pane = Paragraph::new(visible_lines).block(
        Block::default()
            .title(format!(" Agent Events - {:?} ", app.agentstate))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(event_border_color)),
    );

    let input_border_color = if app.focus == Focus::Input {
        Color::Rgb(255, 165, 0)
    } else {
        Color::White
    };

    let cursor = if app.cursor_visible { "█" } else { " " };
    let input_text = format!(
        "{}{}{}",
        &app.inputbuffer[..app.cursor_pos.min(app.inputbuffer.len())],
        cursor,
        &app.inputbuffer[app.cursor_pos.min(app.inputbuffer.len())..]
    );
    let input_pane = Paragraph::new(input_text)
        .block(
            Block::default()
                .title(app.current_session_title.as_str())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(input_border_color)),
        )
        .style(Style::default().fg(Color::LightYellow))
        .wrap(Wrap { trim: false });

    let right_border_style = if app.is_dragging_right_divider || app.hover_right_divider {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Magenta)
    };
    let mut right_lines = Vec::new();
    let core_color = match app.core_kind {
        CoreKind::General => Color::Green,
        CoreKind::Learning => Color::Cyan,
        CoreKind::PairProgramming => Color::Magenta,
    };
    right_lines.push(format!("{}", app.core_kind));
    right_lines.push(String::new());
    right_lines.push("Spaces".to_string());
    match load_spaces_registry() {
        Ok(registry) if registry.spaces.is_empty() => right_lines.push("  none".to_string()),
        Ok(registry) => {
            for space in registry.spaces.iter().take(6) {
                right_lines.push(format!(
                    "  {} [{}] {}",
                    space.id,
                    space.status,
                    context::truncate_middle(space.task.as_deref().unwrap_or("idle"), 36)
                ));
            }
        }
        Err(_) => right_lines.push("  unavailable".to_string()),
    }
    right_lines.push(String::new());
    right_lines.push("LangGraph".to_string());
    match list_langgraph_run_records() {
        Ok(records) if records.is_empty() => right_lines.push("  no runs".to_string()),
        Ok(records) => {
            for record in records.iter().take(4) {
                right_lines.push(format!(
                    "  {} [{}] {}",
                    context::truncate_middle(&record.id, 8),
                    record.status,
                    context::truncate_middle(&record.input, 32)
                ));
            }
        }
        Err(_) => right_lines.push("  unavailable".to_string()),
    }
    right_lines.push(String::new());
    right_lines.push("Scoped Agents".to_string());
    if app.scoped_agents.is_empty() {
        right_lines.push("  none".to_string());
    } else {
        for agent in app.scoped_agents.iter().rev().take(4) {
            right_lines.push(format!(
                "  {} [{}] {}/{}",
                agent.name, agent.status, agent.current_step, agent.max_steps
            ));
        }
    }
    let right_pane = Paragraph::new(right_lines.join("\n"))
        .block(
            Block::default()
                .title(" Runs / Spaces ")
                .borders(Borders::ALL)
                .border_style(right_border_style),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(sidebar, h_chunks[0]);
    frame.render_widget(events_pane, events_area);
    if let Some(scoped_agents_area) = scoped_agents_area {
        let scoped_lines = app
            .scoped_agents
            .iter()
            .rev()
            .take(4)
            .map(|agent| {
                let latest = agent
                    .events
                    .last()
                    .map(String::as_str)
                    .unwrap_or("no events");
                format!(
                    "{} [{}] {}/{} - {}",
                    agent.name,
                    agent.status,
                    agent.current_step,
                    agent.max_steps,
                    context::truncate_middle(latest, 120)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let scoped_pane = Paragraph::new(scoped_lines)
            .block(
                Block::default()
                    .title(" Scoped Agents ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(scoped_pane, scoped_agents_area);
    }
    if app.inputstate == InputState::Settings {
        render_settings_overlay(frame, app, v_chunks[0]);
    }
    frame.render_widget(input_pane, v_chunks[1]);
    frame.render_widget(right_pane, h_chunks[2]);
}

fn push_wrapped_log_lines(
    lines: &mut Vec<Line<'static>>,
    style: Style,
    text: String,
    width: usize,
) {
    let width = width.max(1);

    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(Line::styled(String::new(), style));
            continue;
        }

        let mut current = String::new();
        let mut current_width = 0usize;

        for ch in raw_line.chars() {
            let ch_width = if ch == '\t' { 4 } else { 1 };
            if current_width > 0 && current_width + ch_width > width {
                lines.push(Line::styled(std::mem::take(&mut current), style));
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }

        lines.push(Line::styled(current, style));
    }

    lines.push(Line::raw(""));
}

pub(crate) fn log_entry_to_wrapped_lines(entry: &LogEntry, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match entry {
        LogEntry::User(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Cyan),
            format!("User: {text}"),
            width,
        ),
        LogEntry::Assistant(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::White),
            format!("{text}"),
            width,
        ),
        LogEntry::AssistantFinal(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::White),
            format!(
                "{} FINAL RESPONSE {}\n{}\n{}",
                "▶".repeat(20),
                "◀".repeat(20),
                text,
                "─".repeat(width.min(80)),
            ),
            width,
        ),
        LogEntry::Thinking { kind, text } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::DarkGray),
            format!("Thinking block [{kind}]:\n{text}"),
            width,
        ),
        LogEntry::ToolCall { name, args } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Yellow),
            compact_tool_call_text(name, args),
            width,
        ),
        LogEntry::ToolResult { name, result } => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Green),
            compact_tool_result_text(name, result),
            width,
        ),
        LogEntry::System(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::LightBlue),
            text.clone(),
            width,
        ),
        LogEntry::Error(text) => push_wrapped_log_lines(
            &mut lines,
            Style::default().fg(Color::Red),
            text.clone(),
            width,
        ),
    }
    lines
}

fn render_settings_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let overlay_width = 52.min(area.width.saturating_sub(4));
    let overlay_height = 9.min(area.height.saturating_sub(2));
    if overlay_width < 20 || overlay_height < 5 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let y = area.y + (area.height.saturating_sub(overlay_height)) / 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    frame.render_widget(Clear, overlay_area);

    let field_labels = ["Default Core", "Default Model", "Max Agent Steps"];

    let lines: Vec<Line> = field_labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let is_selected = i == app.settings_selected_field;
            let style = if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            let indicator = if is_selected { " > " } else { "   " };
            let value_str = match i {
                0 => format!("{}", app.settings_draft.default_core),
                1 => {
                    if is_selected {
                        if app.settings_input_buffer.is_empty() {
                            "(none)".to_string()
                        } else {
                            format!("{}█", app.settings_input_buffer)
                        }
                    } else {
                        app.settings_draft
                            .default_model
                            .clone()
                            .unwrap_or_else(|| "(none)".to_string())
                    }
                }
                2 => {
                    if is_selected {
                        if app.settings_input_buffer.is_empty() {
                            "(none)".to_string()
                        } else {
                            format!("{}█", app.settings_input_buffer)
                        }
                    } else {
                        app.settings_draft
                            .max_agent_steps
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "(none)".to_string())
                    }
                }
                _ => String::new(),
            };
            Line::styled(format!("{}{}: {}", indicator, label, value_str), style)
        })
        .collect();

    let settings_pane = Paragraph::new(lines).block(
        Block::default()
            .title(" Settings (Ctrl+S to close) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(settings_pane, overlay_area);
}
