use std::{env, error::Error, path::PathBuf};

use crust_types::CoreKind;
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "crust.config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrustSettings {
    #[serde(default)]
    pub default_core: CoreKind,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub max_agent_steps: Option<u32>,
}

impl Default for CrustSettings {
    fn default() -> Self {
        Self {
            default_core: CoreKind::General,
            default_model: None,
            max_agent_steps: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsManager {
    path: PathBuf,
}

impl SettingsManager {
    pub fn new() -> Self {
        Self {
            path: settings_path(),
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn load(&self) -> Result<CrustSettings, Box<dyn Error + Send + Sync>> {
        if !self.path.exists() {
            return Ok(CrustSettings::default());
        }

        let contents = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn save(&self, settings: &CrustSettings) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(settings)?)?;
        Ok(())
    }
}

impl Default for SettingsManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn settings_path() -> PathBuf {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(SETTINGS_FILE)
}
