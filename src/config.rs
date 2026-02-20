use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub jira_url: String,
    pub email: String,
    pub api_token: String,
    pub download_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            jira_url: String::new(),
            email: String::new(),
            api_token: String::new(),
            download_dir: default_download_dir(),
        }
    }
}

fn default_download_dir() -> PathBuf {
    dirs_sys_path().unwrap_or_else(|| PathBuf::from("C:\\JiraDownloads"))
}

fn dirs_sys_path() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .ok()
        .map(|p| PathBuf::from(p).join("JiraDownloads"))
}

fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| {
        std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
    });
    PathBuf::from(appdata).join("jira-downloader").join("config.json")
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str::<AppConfig>(&data) {
                    return config;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {e}"))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        std::fs::write(&path, data)
            .map_err(|e| format!("Failed to write config: {e}"))?;
        Ok(())
    }
}
