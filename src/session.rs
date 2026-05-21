use openrouter_rs::{OpenRouterClient, api::chat::Message, types::Role};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub max_agent_steps: u32,
    pub modelname: String,
}

impl Config {
    pub fn new() -> Self {
        // Load .env from the current directory, falling back to the project root.
        dotenvy::dotenv().ok();
        dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

        let max_agent_steps = env::var("MAX_AGENT_STEPS")
            .unwrap_or("10".to_string())
            .parse::<u32>()
            .expect("cant parse max agent steps val");

        let modelname = env::var("OPENROUTER_MAIN_MODEL")
            .unwrap_or_else(|_| "moonshotai/kimi-latest".to_string())
            .to_string();

        Config {
            max_agent_steps,
            modelname,
        }
    }

    pub fn client_builder(&self) -> Result<OpenRouterClient, Box<dyn Error + Send + Sync>> {
        let api_key = env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY must be set");
        let client = OpenRouterClient::builder()
            .api_key(api_key.clone())
            .build()
            .expect("Openrouter API Key not valid");
        Ok(client)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub edited_at: String,
    pub messages: Vec<Message>,
    pub sysprompt: Message,
    pub config: Config,
}

impl Session {
    pub fn new(name: String, sysprompt: &str) -> Self {
        let sysprompt: Message = Message::new(Role::System, sysprompt);
        let messages = vec![sysprompt.clone()];
        let now = chrono::Utc::now().to_rfc3339();
        let config = Config::new();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            created_at: now.clone(),
            edited_at: now,
            messages,
            sysprompt,
            config,
        }
    }

    pub fn touch(&mut self) {
        self.edited_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
        self.touch();
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.messages.push(self.sysprompt.clone());
        self.touch();
    }
    pub fn get_config(&self) -> Config {
        self.config.clone()
    }
}
#[derive(Debug, Default, Clone)]
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

    pub fn session_name_exists(&self, session_name: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.name == session_name)
    }

    pub fn create_session(&mut self, name: String, sysprompt: &str) -> &Session {
        let session = Session::new(name, sysprompt);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.current_session_id = Some(id.clone());
        if let Some(session) = self.sessions.get_mut(&id) {
            session.touch();
        }
        self.save_sessions();

        self.get_current_session().unwrap()
    }

    pub fn list_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn list_session_names(&self) -> Vec<&str> {
        self.sessions
            .values()
            .map(|session| session.name.as_str())
            .collect()
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
            self.current_session_id = self.sessions.keys().next().cloned();
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

    pub fn get_current_config(&self) -> Option<Config> {
        if let Some(session) = self.get_current_session() {
            return Some(session.get_config());
        } else {
            return None;
        }
    }

    pub fn clear_current_session(&mut self) {
        if let Some(session) = self.get_current_session_mut() {
            session.clear_messages();
            self.save_sessions();
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
