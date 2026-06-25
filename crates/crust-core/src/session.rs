use crust_types::{Config, CoreKind, Session};
use openrouter_rs::api::chat::Message;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
#[derive(Debug, Default, Clone)]
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    current_session_id: Option<String>,
    storage_path: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let storage_path = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".crust_sessions");

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

    pub fn session_name_exists(&self, session_name: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.name == session_name)
    }

    pub fn create_session(&mut self, name: String, sysprompt: &str) -> &Session {
        self.create_session_with_core(name, sysprompt, CoreKind::default())
    }

    pub fn create_session_with_core(
        &mut self,
        name: String,
        sysprompt: &str,
        core_profile: CoreKind,
    ) -> &Session {
        let mut session = Session::new(name, sysprompt);
        session.core_profile = core_profile;
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.current_session_id = Some(id.clone());
        if let Some(session) = self.sessions.get_mut(&id) {
            session.touch();
        }
        self.save_sessions();

        self.get_current_session().unwrap()
    }

    pub fn list_session_names(&self) -> Vec<&str> {
        self.sessions
            .values()
            .map(|session| session.name.as_str())
            .collect()
    }

    pub fn list_sessions(&self) -> Vec<Session> {
        self.sessions.values().cloned().collect()
    }

    pub fn get_session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    pub fn get_session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }

    pub fn find_session_id_by_name(&self, session_name: &str) -> Option<String> {
        self.sessions
            .values()
            .find(|session| session.name == session_name)
            .map(|session| session.id.clone())
    }

    pub fn switch_session(&mut self, session_id: &str) -> Result<&Session, String> {
        if self.sessions.contains_key(session_id) {
            self.current_session_id = Some(session_id.to_string());
            self.save_sessions();
            Ok(self.get_current_session().unwrap())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    pub fn delete_session(&mut self, session_id: &str) -> Result<(), String> {
        if !self.sessions.contains_key(session_id) {
            return Err(format!("Session '{}' not found", session_id));
        }

        if let Some(current) = &self.current_session_id {
            if current == session_id {
                self.current_session_id = None;
            }
        }

        self.sessions.remove(session_id);
        if self.current_session_id.is_none() {
            self.current_session_id = self
                .sessions
                .values()
                .max_by(|a, b| a.edited_at.cmp(&b.edited_at))
                .map(|session| session.id.clone());
        }
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
            self.save_sessions();
        }
    }

    pub fn add_message_to_session(&mut self, session_id: &str, message: Message) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        session.add_message(message);
        self.save_sessions();
        true
    }

    pub fn record_usage_to_session(
        &mut self,
        session_id: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        session.record_usage(prompt_tokens, completion_tokens, total_tokens);
        self.save_sessions();
        true
    }

    pub fn record_usage_to_current(
        &mut self,
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    ) {
        if let Some(session) = self.get_current_session_mut() {
            session.record_usage(prompt_tokens, completion_tokens, total_tokens);
            self.save_sessions();
        }
    }

    pub fn get_current_config(&self) -> Option<Config> {
        if let Some(session) = self.get_current_session() {
            return Some(session.get_config());
        } else {
            return None;
        }
    }

    pub fn get_session_config(&self, session_id: &str) -> Option<Config> {
        self.sessions.get(session_id).map(Session::get_config)
    }

    pub fn clear_current_session(&mut self) {
        if let Some(session) = self.get_current_session_mut() {
            session.clear_messages();
            self.save_sessions();
        }
    }

    pub fn compact_current_session_to_recent(
        &mut self,
        min_recent_messages: usize,
    ) -> Result<usize, String> {
        let cutoff = self
            .get_current_session_mut()
            .ok_or_else(|| "No current session.".to_string())?
            .compact_to_recent(min_recent_messages)?;
        self.save_sessions();
        Ok(cutoff)
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

    pub fn load_most_recent_session(&mut self) -> bool {
        let sessions_file = self.storage_path.join("sessions.json");

        if !sessions_file.exists() {
            return false;
        }

        let Ok(data) = fs::read_to_string(&sessions_file) else {
            return false;
        };

        let Ok(sessions) = serde_json::from_str::<Vec<Session>>(&data) else {
            return false;
        };

        if sessions.is_empty() {
            return false;
        }

        self.sessions.clear();
        for mut session in sessions {
            if session.edited_at.is_empty() {
                session.edited_at = session.created_at.clone();
            }
            self.sessions.insert(session.id.clone(), session);
        }

        self.current_session_id = self
            .sessions
            .values()
            .max_by(|a, b| a.edited_at.cmp(&b.edited_at))
            .map(|session| session.id.clone());

        self.current_session_id.is_some()
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.save_sessions();
    }
}
