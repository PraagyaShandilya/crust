use std::{convert::Infallible, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Sse, sse::Event},
    routing::{get, post},
};
use crust_core::{SessionManager, agent_main_run};
use crust_types::{AgentEvent, Session, TaggedAgentEvent};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast, mpsc},
};

const DEFAULT_SYSPROMPT: &str = "You are Crust, a helpful coding agent.";

#[derive(Clone)]
struct AppState {
    sessions: Arc<Mutex<SessionManager>>,
    events: broadcast::Sender<TaggedAgentEvent>,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session_manager = SessionManager::new();
    session_manager.load_most_recent_session();

    let (events, _) = broadcast::channel(1024);
    let state = AppState {
        sessions: Arc::new(Mutex::new(session_manager)),
        events,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/sessions/{id}/messages", post(post_message))
        .route("/api/sessions/{id}/events", get(session_events))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:3030").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn list_sessions(State(state): State<AppState>) -> Json<Vec<Session>> {
    let sessions = state.sessions.lock().await;
    Json(sessions.list_sessions())
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
    Json(sessions.create_session(name, &sysprompt).clone())
}

async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let sessions = state.sessions.lock().await;
    match sessions.get_session(&id).cloned() {
        Some(session) => Json(session).into_response(),
        None => not_found(id),
    }
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut sessions = state.sessions.lock().await;
    match sessions.delete_session(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => not_found(id),
    }
}

async fn post_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<MessageRequest>,
) -> impl IntoResponse {
    if request.prompt.trim().is_empty() {
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
            return not_found(id);
        }
    }

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
            let _ = events.send(TaggedAgentEvent {
                session_id: session_id.clone(),
                event: AgentEvent::Error {
                    message: err.to_string(),
                },
            });
        }

        let _ = events.send(TaggedAgentEvent {
            session_id,
            event: AgentEvent::Finished,
        });
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

fn not_found(id: String) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("session '{id}' not found"),
        }),
    )
        .into_response()
}
