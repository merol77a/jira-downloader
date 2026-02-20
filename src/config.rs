use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use winreg::{enums::*, RegKey};

const REG_KEY_PATH: &str = "Software\\jira-downloader";
const REG_ENC_VALUE: &str = "encryption_key";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub jira_url: String,
    pub email: String,
    /// Plaintext token — never written to disk.
    #[serde(skip)]
    pub api_token: String,
    pub download_dir: PathBuf,
    /// AES-256-GCM encrypted token stored in config.json.
    #[serde(default)]
    api_token_enc: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            jira_url: String::new(),
            email: String::new(),
            api_token: String::new(),
            download_dir: default_download_dir(),
            api_token_enc: String::new(),
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

/// Returns the 32-byte AES key stored in the registry, generating one on first run.
fn get_or_create_key() -> Result<[u8; 32], String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (subkey, _) = hkcu
        .create_subkey(REG_KEY_PATH)
        .map_err(|e| format!("Registry open error: {e}"))?;

    // Try to read an existing key.
    if let Ok(encoded) = subkey.get_value::<String, _>(REG_ENC_VALUE) {
        if let Ok(bytes) = B64.decode(&encoded) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Ok(arr);
            }
        }
    }

    // First run — generate and persist a new key.
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let encoded = B64.encode(key);
    subkey
        .set_value(REG_ENC_VALUE, &encoded)
        .map_err(|e| format!("Registry write error: {e}"))?;
    Ok(key)
}

fn encrypt_token(token: &str) -> Result<String, String> {
    let key_bytes = get_or_create_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, token.as_bytes())
        .map_err(|e| format!("Encryption error: {e}"))?;

    // Encode as base64(nonce ++ ciphertext)
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(B64.encode(combined))
}

fn decrypt_token(encoded: &str) -> Option<String> {
    let key_bytes = get_or_create_key().ok()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));

    let combined = B64.decode(encoded).ok()?;
    if combined.len() < 13 {
        return None;
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
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

        // Decrypt token from the stored encrypted blob.
        if !config.api_token_enc.is_empty() {
            if let Some(token) = decrypt_token(&config.api_token_enc) {
                config.api_token = token;
            }
        }

        config
    }

    pub fn save(&self) -> Result<(), String> {
        let mut on_disk = self.clone();

        // Encrypt the plaintext token for storage.
        if !self.api_token.is_empty() {
            on_disk.api_token_enc = encrypt_token(&self.api_token)?;
        } else {
            on_disk.api_token_enc = String::new();
        }

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
}
