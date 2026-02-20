use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "jira-downloader";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub jira_url: String,
    pub email: String,
    // Never written to disk — stored in Windows Credential Manager instead
    #[serde(skip)]
    pub api_token: String,
    pub download_dir: PathBuf,
    // Tracks which email the stored credential belongs to,
    // so we can clean up when the email changes.
    #[serde(default)]
    credential_user: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            jira_url: String::new(),
            email: String::new(),
            api_token: String::new(),
            download_dir: default_download_dir(),
            credential_user: String::new(),
        }
    }
}

fn default_download_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .ok()
        .map(|p| PathBuf::from(p).join("JiraDownloads"))
        .unwrap_or_else(|| PathBuf::from("C:\\JiraDownloads"))
}

fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    PathBuf::from(appdata)
        .join("jira-downloader")
        .join("config.json")
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        let mut config = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|data| serde_json::from_str::<AppConfig>(&data).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        };

        // Load token from Windows Credential Manager
        let lookup_user = if config.credential_user.is_empty() {
            config.email.clone()
        } else {
            config.credential_user.clone()
        };

        if !lookup_user.is_empty() {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &lookup_user) {
                if let Ok(token) = entry.get_password() {
                    config.api_token = token;
                }
            }
        }

        config
    }

    pub fn save(&self) -> Result<(), String> {
        // --- 1. Handle credential rotation if email changed ---
        if !self.credential_user.is_empty()
            && self.credential_user != self.email
        {
            // Delete the old credential silently
            if let Ok(old_entry) =
                keyring::Entry::new(KEYRING_SERVICE, &self.credential_user)
            {
                let _ = old_entry.delete_credential();
            }
        }

        // --- 2. Store token in Windows Credential Manager ---
        if !self.email.is_empty() && !self.api_token.is_empty() {
            let entry = keyring::Entry::new(KEYRING_SERVICE, &self.email)
                .map_err(|e| format!("Credential Manager error: {e}"))?;
            entry
                .set_password(&self.api_token)
                .map_err(|e| format!("Failed to save token to Credential Manager: {e}"))?;
        }

        // --- 3. Write JSON config (token excluded via #[serde(skip)]) ---
        let mut on_disk = self.clone();
        on_disk.credential_user = self.email.clone(); // remember which user owns the cred

        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {e}"))?;
        }
        let data = serde_json::to_string_pretty(&on_disk)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        std::fs::write(&path, data)
            .map_err(|e| format!("Failed to write config: {e}"))?;

        Ok(())
    }

    /// Remove the stored credential from Windows Credential Manager.
    #[allow(dead_code)]
    pub fn delete_credential(&self) {
        let user = if self.credential_user.is_empty() {
            &self.email
        } else {
            &self.credential_user
        };
        if !user.is_empty() {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, user) {
                let _ = entry.delete_credential();
            }
        }
    }
}
