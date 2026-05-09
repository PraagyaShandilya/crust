use openrouter_rs::{
    Content, OpenRouterClient,
    api::chat::Message,
    types::{Role, ToolCall, typed_tool::TypedTool},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub messages: Vec<Message>,
    pub sysprompt: Message,
}

impl Session {
    pub fn new(name: String, sysprompt: &str) -> Self {
        let sysprompt: Message = Message::new(Role::System, sysprompt);
        let messages = vec![sysprompt.clone()];
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            created_at: chrono::Utc::now().to_rfc3339(),
            messages: messages,
            sysprompt: sysprompt,
        }
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    current_session_id: Option<String>,
    storage_path: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let storage_path = PathBuf::from(".crust_sessions");

        // Create storage directory if it doesn't exist
        if !storage_path.exists() {
            fs::create_dir_all(&storage_path).unwrap_or_else(|e| {
                eprintln!("Warning: Could not create session storage directory: {}", e);
            });
        }

        Self {
            sessions: HashMap::new(),
            current_session_id: None,
            storage_path,
        }
    }

    pub fn create_session(&mut self, name: String, sysprompt: &str) -> &Session {
        let session = Session::new(name, sysprompt);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.current_session_id = Some(id);
        self.save_sessions();

        self.get_current_session().unwrap()
    }

    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn switch_session(&mut self, session_id: &str) -> Result<&Session, String> {
        if self.sessions.contains_key(session_id) {
            self.current_session_id = Some(session_id.to_string());
            Ok(self.get_current_session().unwrap())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<(), String> {
        if let Some(current) = &self.current_session_id {
            if current == session_id {
                self.current_session_id = None;
            }
        }
        self.sessions.remove(session_id);
        self.save_sessions();
        Ok(())
    }

    pub fn get_current_session(&self) -> Option<&Session> {
        self.current_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    pub fn get_current_session_mut(&mut self) -> Option<&mut Session> {
        self.current_session_id
            .as_ref()
            .and_then(|id| self.sessions.get_mut(id))
    }

    pub fn add_message_to_current(&mut self, message: Message) {
        if let Some(session) = self.get_current_session_mut() {
            session.add_message(message);
        }
    }

    pub fn clear_current_session(&mut self) {
        if let Some(session) = self.get_current_session_mut() {
            session.clear_messages();
        }
    }

    fn save_sessions(&self) {
        let sessions_file = self.storage_path.join("sessions.json");
        let sessions_data: Vec<&Session> = self.sessions.values().collect();

        if let Ok(json) = serde_json::to_string_pretty(&sessions_data) {
            if let Err(e) = fs::write(&sessions_file, json) {
                eprintln!("Warning: Could not save sessions: {}", e);
            }
        }
    }

    fn load_sessions(&mut self) {
        let sessions_file = self.storage_path.join("sessions.json");

        if sessions_file.exists() {
            if let Ok(data) = fs::read_to_string(&sessions_file) {
                if let Ok(sessions) = serde_json::from_str::<Vec<Session>>(&data) {
                    for session in sessions.clone() {
                        self.sessions.insert(session.id.clone(), session);
                    }
                    // Restore current session if it exists
                    if let Some(first_session) = sessions.first() {
                        self.current_session_id = Some(first_session.id.clone());
                    }
                }
            }
        }
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.save_sessions();
    }
}
