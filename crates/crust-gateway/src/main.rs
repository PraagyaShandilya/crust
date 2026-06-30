use std::{
    convert::Infallible,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Sse, sse::Event},
    routing::{get, post},
};
use crust_core::{
    ApprovalQueue, CrustSettings, InteractionMode, SessionManager, SettingsManager,
    agent_main_run, core_profile,
};
use crust_types::{AgentEvent, CoreKind, Session, TaggedAgentEvent};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast, mpsc},
};

const DEFAULT_SYSPROMPT: &str = "You are Crust, a helpful coding agent.";
const BIND_ADDR: &str = "127.0.0.1:3030";

#[derive(Clone)]
struct AppState {
    sessions: Arc<Mutex<SessionManager>>,
    events: broadcast::Sender<TaggedAgentEvent>,
    approvals: ApprovalQueue,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
struct CreateSessionRequest {
    name: Option<String>,
    sysprompt: Option<String>,
}

#[derive(Deserialize)]
struct MessageRequest {
    prompt: String,
}

#[derive(Serialize)]
struct MessageAcceptedResponse {
    session_id: String,
    status: &'static str,
}

#[derive(Serialize)]
struct CoreResponse {
    kind: CoreKind,
    display_name: &'static str,
    description: &'static str,
    interaction_mode: &'static str,
    default_model: Option<String>,
}

#[derive(Serialize)]
struct ModelResponse {
    id: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct SkillResponse {
    name: String,
    description: String,
}

#[derive(Serialize)]
struct PendingApprovalResponse {
    id: String,
    session_id: String,
    tool_name: String,
    args: String,
}

#[derive(Deserialize)]
struct RejectRequest {
    reason: String,
}

#[derive(Deserialize)]
struct RenameSessionRequest {
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session_manager = SessionManager::new();
    let restored_session = session_manager.load_most_recent_session();

    let (events, _) = broadcast::channel(1024);
    let state = AppState {
        sessions: Arc::new(Mutex::new(session_manager)),
        events,
        approvals: ApprovalQueue::default(),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/{id}",
            get(get_session)
                .patch(rename_session)
                .delete(delete_session),
        )
        .route("/api/sessions/{id}/messages", post(post_message))
        .route("/api/sessions/{id}/events", get(session_events))
        .route("/api/sessions/{id}/clear", post(clear_session))
        .route("/api/sessions/{id}/compact", post(compact_session))
        .route("/api/sessions/{id}/context", get(get_context))
        .route("/api/sessions/{id}/approvals", get(list_approvals))
        .route(
            "/api/sessions/{id}/approvals/{approval_id}/approve",
            post(approve_approval),
        )
        .route(
            "/api/sessions/{id}/approvals/{approval_id}/reject",
            post(reject_approval),
        )
        .route("/api/cores", get(list_cores))
        .route("/api/models", get(list_models))
        .route("/api/settings", get(get_settings).put(update_settings))
        .route("/api/spaces", get(list_spaces).post(create_space))
        .route("/api/spaces/{id}", get(get_space))
        .route("/api/spaces/{id}/stop", post(stop_space))
        .route("/api/skills", get(list_skills))
        .with_state(state);

    let listener = TcpListener::bind(BIND_ADDR).await?;
    print_startup_banner(restored_session);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<HealthResponse> {
    log_activity("http", "GET /healthz -> 200");
    Json(HealthResponse { status: "ok" })
}

async fn list_sessions(State(state): State<AppState>) -> Json<Vec<Session>> {
    let sessions = state.sessions.lock().await;
    let sessions = sessions.list_sessions();
    log_activity(
        "http",
        &format!("GET /api/sessions -> 200 ({} sessions)", sessions.len()),
    );
    Json(sessions)
}

async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Json<Session> {
    let name = request
        .name
        .unwrap_or_else(|| "Gateway Session".to_string());
    let sysprompt = request
        .sysprompt
        .unwrap_or_else(|| DEFAULT_SYSPROMPT.to_string());
    let mut sessions = state.sessions.lock().await;
    let session = sessions.create_session(name, &sysprompt).clone();
    log_activity(
        "http",
        &format!("POST /api/sessions -> 200 ({})", session.id),
    );
    Json(session)
}

async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let sessions = state.sessions.lock().await;
    match sessions.get_session(&id).cloned() {
        Some(session) => {
            log_activity("http", &format!("GET /api/sessions/{id} -> 200"));
            Json(session).into_response()
        }
        None => {
            log_activity("http", &format!("GET /api/sessions/{id} -> 404"));
            not_found(id)
        }
    }
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut sessions = state.sessions.lock().await;
    match sessions.delete_session(&id) {
        Ok(()) => {
            log_activity("http", &format!("DELETE /api/sessions/{id} -> 204"));
            StatusCode::NO_CONTENT.into_response()
        }
        Err(_) => {
            log_activity("http", &format!("DELETE /api/sessions/{id} -> 404"));
            not_found(id)
        }
    }
}

async fn rename_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RenameSessionRequest>,
) -> impl IntoResponse {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        log_activity("http", &format!("PATCH /api/sessions/{id} -> 400"));
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "name must not be empty".to_string(),
            }),
        )
            .into_response();
    }

    let mut sessions = state.sessions.lock().await;
    match sessions.rename_session(&id, name) {
        Ok(()) => {
            let session = sessions.get_session(&id).cloned().unwrap();
            log_activity("http", &format!("PATCH /api/sessions/{id} -> 200"));
            Json(session).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("PATCH /api/sessions/{id} -> 400"));
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: err }),
            )
                .into_response()
        }
    }
}

async fn clear_session(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let mut sessions = state.sessions.lock().await;
    if sessions.clear_session(&id) {
        log_activity("http", &format!("POST /api/sessions/{id}/clear -> 200"));
        Json(serde_json::json!({ "status": "cleared" })).into_response()
    } else {
        log_activity("http", &format!("POST /api/sessions/{id}/clear -> 404"));
        not_found(id)
    }
}

async fn compact_session(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let mut sessions = state.sessions.lock().await;
    match sessions.compact_session_to_recent(&id, 10) {
        Ok(cutoff) => {
            log_activity("http", &format!("POST /api/sessions/{id}/compact -> 200 (cutoff {cutoff})"));
            Json(serde_json::json!({ "status": "compacted", "cutoff": cutoff })).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("POST /api/sessions/{id}/compact -> 400"));
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: err }),
            )
                .into_response()
        }
    }
}

async fn get_context(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let sessions = state.sessions.lock().await;
    let Some(session) = sessions.get_session(&id) else {
        log_activity("http", &format!("GET /api/sessions/{id}/context -> 404"));
        return not_found(id);
    };
    let config = session.get_config();
    let context_builder = crust_core::ContextBuilder::default();
    let context = context_builder.build_context(session, &config);
    let estimated_tokens = context_builder.estimate_context_tokens(&context);
    let context_window = context_builder.context_window_tokens(&config);
    let estimated_ratio = context_builder.estimated_context_ratio(&context, &config);
    let last_api_ratio = context_builder
        .context_ratio_from_api_usage(session.latest_prompt_tokens, &config);
    log_activity("http", &format!("GET /api/sessions/{id}/context -> 200"));
    Json(serde_json::json!({
        "model": config.modelname,
        "context_window": context_window,
        "context_messages": context.len(),
        "estimated_tokens": estimated_tokens,
        "estimated_ratio": estimated_ratio,
        "last_prompt_tokens": session.latest_prompt_tokens,
        "last_api_ratio": last_api_ratio,
        "has_summary": session.summary.is_some(),
        "compacted_until": session.compacted_until,
        "total_messages": session.messages.len(),
    }))
    .into_response()
}

async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    if request.prompt.trim().is_empty() {
        log_activity("http", &format!("POST /api/sessions/{id}/messages -> 400"));
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "prompt must not be empty".to_string(),
            }),
        )
            .into_response();
    }

    {
        let sessions = state.sessions.lock().await;
        if sessions.get_session(&id).is_none() {
            log_activity("http", &format!("POST /api/sessions/{id}/messages -> 404"));
            return not_found(id);
        }
    }

    log_activity(
        "agent",
        &format!(
            "accepted prompt for session {id} ({} chars)",
            request.prompt.len()
        ),
    );

    let _ = state.events.send(TaggedAgentEvent {
        session_id: id.clone(),
        event: AgentEvent::UserSubmitted {
            prompt: request.prompt.clone(),
        },
    });

    let (agent_tx, mut agent_rx) = mpsc::channel(128);
    let events = state.events.clone();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            log_agent_event(&event);
            let _ = events.send(event);
        }
    });

    let sessions = state.sessions.clone();
    let events = state.events.clone();
    let session_id = id.clone();
    tokio::spawn(async move {
        if let Err(err) =
            agent_main_run(sessions, request.prompt, session_id.clone(), agent_tx).await
        {
            log_activity("agent", &format!("session {session_id} error: {err}"));
            let _ = events.send(TaggedAgentEvent {
                session_id: session_id.clone(),
                event: AgentEvent::Error {
                    message: err.to_string(),
                },
            });
        }

        let _ = events.send(TaggedAgentEvent {
            session_id: session_id.clone(),
            event: AgentEvent::Finished,
        });
        log_activity("agent", &format!("session {session_id} finished"));
    });

    (
        StatusCode::ACCEPTED,
        Json(MessageAcceptedResponse {
            session_id: id,
            status: "accepted",
        }),
    )
        .into_response()
}

async fn session_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    log_activity("sse", &format!("client connected for session {id}"));
    let rx = state.events.subscribe();
    let events = stream::unfold(rx, move |mut rx| {
        let session_id = id.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(event) if event.session_id == session_id => {
                        let data = serde_json::to_string(&event).unwrap_or_else(|err| {
                            format!(r#"{{"error":"failed to serialize event: {err}"}}"#)
                        });
                        return Some((Ok(Event::default().event("agent_event").data(data)), rx));
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });

    Sse::new(events)
}

async fn list_cores() -> Json<Vec<CoreResponse>> {
    let cores: Vec<CoreResponse> = CoreKind::all()
        .iter()
        .map(|kind| {
            let profile = core_profile(*kind);
            CoreResponse {
                kind: profile.kind,
                display_name: profile.display_name,
                description: profile.description,
                interaction_mode: interaction_mode_str(profile.interaction_mode),
                default_model: profile.default_model.map(String::from),
            }
        })
        .collect();
    log_activity(
        "http",
        &format!("GET /api/cores -> 200 ({} cores)", cores.len()),
    );
    Json(cores)
}

async fn list_models() -> Json<Vec<ModelResponse>> {
    let models: Vec<ModelResponse> = crust_core::models_generated::OPENROUTER_MODELS
        .iter()
        .map(|m| ModelResponse {
            id: m.id,
            name: m.name,
        })
        .collect();
    log_activity(
        "http",
        &format!("GET /api/models -> 200 ({} models)", models.len()),
    );
    Json(models)
}

async fn get_settings() -> impl IntoResponse {
    match SettingsManager::new().load() {
        Ok(settings) => {
            log_activity("http", "GET /api/settings -> 200");
            Json(settings).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("GET /api/settings -> 500 ({err})"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: err.to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn update_settings(Json(settings): Json<CrustSettings>) -> impl IntoResponse {
    match SettingsManager::new().save(&settings) {
        Ok(()) => {
            log_activity("http", "PUT /api/settings -> 200");
            Json(settings).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("PUT /api/settings -> 500 ({err})"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: err.to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn list_spaces() -> impl IntoResponse {
    match crust_core::spaces::load_spaces_registry() {
        Ok(registry) => {
            log_activity(
                "http",
                &format!("GET /api/spaces -> 200 ({} spaces)", registry.spaces.len()),
            );
            Json(registry).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("GET /api/spaces -> 500 ({err})"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: err }),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize)]
struct CreateSpaceRequest {
    id: String,
}

async fn create_space(Json(request): Json<CreateSpaceRequest>) -> impl IntoResponse {
    let id = request.id.trim().to_string();
    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "id must not be empty".to_string(),
            }),
        )
            .into_response();
    }

    let mut registry = match crust_core::spaces::load_spaces_registry() {
        Ok(r) => r,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: err }),
            )
                .into_response();
        }
    };

    if registry.find(&id).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!("space '{id}' already exists"),
            }),
        )
            .into_response();
    }

    let now = chrono::Utc::now().to_rfc3339();
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();
    let space = crust_types::CrustSpace {
        id: id.clone(),
        name: id.clone(),
        session_id: uuid::Uuid::new_v4().to_string(),
        cwd,
        status: crust_types::SpaceStatus::Idle,
        process_id: None,
        task_id: None,
        task: None,
        created_at: now.clone(),
        updated_at: now,
    };
    registry.upsert(space.clone());

    if let Err(err) = crust_core::spaces::save_spaces_registry(&registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: err.to_string(),
            }),
        )
            .into_response();
    }

    log_activity("http", &format!("POST /api/spaces -> 201 ({id})"));
    (
        StatusCode::CREATED,
        Json(space),
    )
        .into_response()
}

async fn get_space(Path(id): Path<String>) -> impl IntoResponse {
    match crust_core::spaces::load_spaces_registry() {
        Ok(registry) => match registry.find(&id).cloned() {
            Some(space) => {
                log_activity("http", &format!("GET /api/spaces/{id} -> 200"));
                Json(space).into_response()
            }
            None => {
                log_activity("http", &format!("GET /api/spaces/{id} -> 404"));
                not_found(id)
            }
        },
        Err(err) => {
            log_activity("http", &format!("GET /api/spaces/{id} -> 500"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: err }),
            )
                .into_response()
        }
    }
}

async fn stop_space(Path(id): Path<String>) -> impl IntoResponse {
    let mut registry = match crust_core::spaces::load_spaces_registry() {
        Ok(r) => r,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: err }),
            )
                .into_response();
        }
    };

    match registry.find_mut(&id) {
        Some(space) => {
            space.status = crust_types::SpaceStatus::Stopped;
            space.updated_at = chrono::Utc::now().to_rfc3339();
            if let Err(err) = crust_core::spaces::save_spaces_registry(&registry) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: err.to_string(),
                    }),
                )
                    .into_response();
            }
            log_activity("http", &format!("POST /api/spaces/{id}/stop -> 200"));
            Json(serde_json::json!({ "status": "stopped" })).into_response()
        }
        None => {
            log_activity("http", &format!("POST /api/spaces/{id}/stop -> 404"));
            not_found(id)
        }
    }
}

async fn list_skills() -> impl IntoResponse {
    match crust_core::skills::load_markdown_skills() {
        Ok(skills) => {
            let response: Vec<SkillResponse> = skills
                .iter()
                .map(|s| SkillResponse {
                    name: s.name.clone(),
                    description: s.description.clone(),
                })
                .collect();
            log_activity(
                "http",
                &format!("GET /api/skills -> 200 ({} skills)", response.len()),
            );
            Json(response).into_response()
        }
        Err(err) => {
            log_activity("http", &format!("GET /api/skills -> 500 ({err})"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: err }),
            )
                .into_response()
        }
    }
}

async fn list_approvals(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Vec<PendingApprovalResponse>> {
    let pending = state.approvals.pending_for_session(&id).await;
    let response: Vec<PendingApprovalResponse> = pending
        .iter()
        .map(|p| PendingApprovalResponse {
            id: p.id.clone(),
            session_id: p.session_id.clone(),
            tool_name: p.tool_name.clone(),
            args: p.args.clone(),
        })
        .collect();
    log_activity(
        "http",
        &format!(
            "GET /api/sessions/{id}/approvals -> 200 ({} pending)",
            response.len()
        ),
    );
    Json(response)
}

async fn approve_approval(
    State(state): State<AppState>,
    Path((id, approval_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let found = state.approvals.approve(&approval_id).await;
    if found {
        log_activity(
            "http",
            &format!("POST /api/sessions/{id}/approvals/{approval_id}/approve -> 200"),
        );
        StatusCode::OK.into_response()
    } else {
        log_activity(
            "http",
            &format!("POST /api/sessions/{id}/approvals/{approval_id}/approve -> 404"),
        );
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("approval '{approval_id}' not found"),
            }),
        )
            .into_response()
    }
}

async fn reject_approval(
    State(state): State<AppState>,
    Path((id, approval_id)): Path<(String, String)>,
    Json(request): Json<RejectRequest>,
) -> impl IntoResponse {
    let found = state.approvals.reject(&approval_id, request.reason).await;
    if found {
        log_activity(
            "http",
            &format!("POST /api/sessions/{id}/approvals/{approval_id}/reject -> 200"),
        );
        StatusCode::OK.into_response()
    } else {
        log_activity(
            "http",
            &format!("POST /api/sessions/{id}/approvals/{approval_id}/reject -> 404"),
        );
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("approval '{approval_id}' not found"),
            }),
        )
            .into_response()
    }
}

fn interaction_mode_str(mode: InteractionMode) -> &'static str {
    match mode {
        InteractionMode::Autonomous => "autonomous",
        InteractionMode::SuggestAndConfirm => "suggest_and_confirm",
    }
}

fn not_found(id: String) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("session '{id}' not found"),
        }),
    )
        .into_response()
}

fn print_startup_banner(restored_session: bool) {
    println!("Crust gateway");
    println!("  bind:     http://{BIND_ADDR}");
    println!("  health:   http://{BIND_ADDR}/healthz");
    println!("  sessions: http://{BIND_ADDR}/api/sessions");
    println!("  restored: {restored_session}");
    println!("\nActivity");
}

fn log_activity(component: &str, message: &str) {
    println!("[{}] {:<7} {message}", timestamp(), component);
}

fn log_agent_event(event: &TaggedAgentEvent) {
    let summary = match &event.event {
        AgentEvent::UserSubmitted { prompt } => format!("user submitted ({} chars)", prompt.len()),
        AgentEvent::Thinking { kind, text } => format!("thinking {kind} ({} chars)", text.len()),
        AgentEvent::ToolCallStarted { name, .. } => format!("tool started {name}"),
        AgentEvent::ToolCallFinished { name, result } => {
            format!("tool finished {name} ({} chars)", result.len())
        }
        AgentEvent::AssistantDelta { text } => format!("assistant delta ({} chars)", text.len()),
        AgentEvent::AssistantFinal { text } => format!("assistant final ({} chars)", text.len()),
        AgentEvent::Error { message } => format!("error {message}"),
        AgentEvent::SystemNotice { message } => format!("notice {message}"),
        AgentEvent::MaxStepsReached => "max steps reached".to_string(),
        AgentEvent::SessionTitleUpdated { title } => format!("title updated {title}"),
        AgentEvent::ScopedAgentStarted { id, name, .. } => {
            format!("scoped agent started {name} ({id})")
        }
        AgentEvent::ScopedAgentStep { id, step } => format!("scoped agent {id} step {step}"),
        AgentEvent::ScopedAgentStatus { id, status } => {
            format!("scoped agent {id} status {status}")
        }
        AgentEvent::ScopedAgentLog { id, message } => format!("scoped agent {id} log {message}"),
        AgentEvent::ScopedAgentFinished { id, result } => {
            format!("scoped agent {id} finished ({} chars)", result.len())
        }
        AgentEvent::ScopedAgentError { id, message } => {
            format!("scoped agent {id} error {message}")
        }
        AgentEvent::Finished => "finished".to_string(),
    };
    log_activity("agent", &format!("session {}: {summary}", event.session_id));
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    seconds.to_string()
}
