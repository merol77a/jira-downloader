use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::jira::Attachment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFile {
    pub issue_key: String,
    pub issue_summary: String,
    pub issue_status: String,
    pub last_checked: DateTime<Utc>,
    pub marked_for_deletion: bool,
}

impl ControlFile {
    pub fn new(key: &str, summary: &str, status: &str) -> Self {
        Self {
            issue_key: key.to_string(),
            issue_summary: summary.to_string(),
            issue_status: status.to_string(),
            last_checked: Utc::now(),
            marked_for_deletion: false,
        }
    }

    pub fn is_closed(&self) -> bool {
        let s = self.issue_status.to_lowercase();
        s == "done" || s == "closed" || s == "resolved" || s.contains("clos") || s.contains("resolv")
    }
}

#[derive(Debug, Clone)]
pub struct IncidentFolder {
    #[allow(dead_code)]
    pub path: PathBuf,
    pub control: ControlFile,
    pub folder_size: u64,
}

pub struct StorageManager {
    pub base_dir: PathBuf,
}

impl StorageManager {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn issue_dir(&self, issue_key: &str) -> PathBuf {
        self.base_dir.join(issue_key)
    }

    #[allow(dead_code)]
    pub fn control_file_path(&self, issue_key: &str) -> PathBuf {
        self.issue_dir(issue_key).join(".jira_control.json")
    }

    pub fn save_control_file(&self, ctrl: &ControlFile) -> Result<(), String> {
        let dir = self.issue_dir(&ctrl.issue_key);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create issue dir: {e}"))?;
        let path = dir.join(".jira_control.json");
        let data = serde_json::to_string_pretty(ctrl)
            .map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(&path, data)
            .map_err(|e| format!("Write error: {e}"))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn load_control_file(&self, issue_key: &str) -> Option<ControlFile> {
        let path = self.control_file_path(issue_key);
        if !path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn attachment_exists(&self, issue_key: &str, attachment: &Attachment) -> bool {
        let date_str = attachment.created.format("%Y-%m-%d").to_string();
        self.issue_dir(issue_key)
            .join(&date_str)
            .join(&attachment.filename)
            .exists()
    }

    pub fn save_attachment(
        &self,
        issue_key: &str,
        attachment: &Attachment,
        data: &bytes::Bytes,
    ) -> Result<PathBuf, String> {
        let date_str = attachment.created.format("%Y-%m-%d").to_string();
        let date_dir = self.issue_dir(issue_key).join(&date_str);
        std::fs::create_dir_all(&date_dir)
            .map_err(|e| format!("Failed to create date dir: {e}"))?;

        let target_path = resolve_conflict(&date_dir, &attachment.filename);
        std::fs::write(&target_path, data.as_ref())
            .map_err(|e| format!("Failed to write file: {e}"))?;
        Ok(target_path)
    }

    /// Scan base_dir for folders that contain .jira_control.json
    pub fn scan_incidents(&self) -> Vec<IncidentFolder> {
        let mut result = Vec::new();
        let read_dir = match std::fs::read_dir(&self.base_dir) {
            Ok(rd) => rd,
            Err(_) => return result,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let ctrl_path = path.join(".jira_control.json");
            if !ctrl_path.exists() {
                continue;
            }
            if let Ok(data) = std::fs::read_to_string(&ctrl_path) {
                if let Ok(ctrl) = serde_json::from_str::<ControlFile>(&data) {
                    let size = dir_size(&path);
                    result.push(IncidentFolder {
                        path,
                        control: ctrl,
                        folder_size: size,
                    });
                }
            }
        }

        result.sort_by(|a, b| a.control.issue_key.cmp(&b.control.issue_key));
        result
    }

    /// Returns the latest date subfolder (YYYY-MM-DD) inside the issue dir,
    /// or the issue dir itself if no date subfolders exist yet.
    pub fn latest_date_folder(&self, issue_key: &str) -> PathBuf {
        let issue_dir = self.issue_dir(issue_key);
        let mut date_dirs: Vec<PathBuf> = Vec::new();

        if let Ok(rd) = std::fs::read_dir(&issue_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                // Accept YYYY-MM-DD named dirs only
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.len() == 10
                        && name.as_bytes().get(4) == Some(&b'-')
                        && name.as_bytes().get(7) == Some(&b'-')
                    {
                        date_dirs.push(p);
                    }
                }
            }
        }

        date_dirs.sort();
        date_dirs.last().cloned().unwrap_or(issue_dir)
    }

    pub fn open_path(path: &Path) {
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(path).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(path).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }

    pub fn delete_folder(&self, issue_key: &str) -> Result<(), String> {
        let dir = self.issue_dir(issue_key);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("Failed to delete folder: {e}"))?;
        }
        Ok(())
    }

    pub fn open_folder(&self, issue_key: &str) {
        let dir = self.issue_dir(issue_key);
        if dir.exists() {
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("explorer")
                    .arg(dir.to_str().unwrap_or("."))
                    .spawn();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg(&dir).spawn();
            }
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            }
        }
    }
}

fn resolve_conflict(dir: &Path, filename: &str) -> PathBuf {
    let path = dir.join(filename);
    if !path.exists() {
        return path;
    }

    // Split filename into stem and extension
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let mut counter = 2u32;
    loop {
        let candidate = dir.join(format!("{stem}_{counter}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                total += p.metadata().map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                total += dir_size(&p);
            }
        }
    }
    total
}
