use std::sync::{Arc, Mutex};

use egui;

use crate::config::AppConfig;
use crate::jira::{Attachment, JiraClient};
use crate::storage::StorageManager;

#[derive(Debug, Clone)]
pub enum FileState {
    Pending,
    Downloading { downloaded: u64, total: u64 },
    Done,
    AlreadyOnDisk,
    Error(String),
}

impl FileState {
    pub fn progress_fraction(&self) -> Option<f32> {
        match self {
            FileState::Downloading { downloaded, total } if *total > 0 => {
                Some(*downloaded as f32 / *total as f32)
            }
            FileState::Done | FileState::AlreadyOnDisk => Some(1.0),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            FileState::Pending => "Pending".to_string(),
            FileState::Downloading { downloaded, total } => {
                if *total > 0 {
                    let pct = (*downloaded as f32 / *total as f32 * 100.0) as u32;
                    format!("{pct}%")
                } else {
                    format!("{} B", downloaded)
                }
            }
            FileState::Done => "Done ✓".to_string(),
            FileState::AlreadyOnDisk => "On disk ✓".to_string(),
            FileState::Error(e) => format!("Error: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub attachment: Attachment,
    pub state: Arc<Mutex<FileState>>,
    pub selected: bool,
}

impl DownloadItem {
    pub fn new(attachment: Attachment) -> Self {
        Self {
            attachment,
            state: Arc::new(Mutex::new(FileState::Pending)),
            selected: true,
        }
    }

    pub fn current_state(&self) -> FileState {
        self.state.lock().unwrap().clone()
    }
}

pub struct DownloadManager {
    runtime: Arc<tokio::runtime::Runtime>,
}

impl DownloadManager {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>) -> Self {
        Self { runtime }
    }

    pub fn start_download(
        &self,
        item: &DownloadItem,
        issue_key: &str,
        config: &AppConfig,
        ctx: egui::Context,
    ) {
        let attachment = item.attachment.clone();
        let state = Arc::clone(&item.state);
        let issue_key = issue_key.to_string();
        let config = config.clone();

        self.runtime.spawn(async move {
            {
                let mut s = state.lock().unwrap();
                *s = FileState::Downloading {
                    downloaded: 0,
                    total: attachment.size,
                };
            }
            ctx.request_repaint();

            let client = JiraClient::new(config.clone());
            let state_clone = Arc::clone(&state);
            let ctx_clone = ctx.clone();

            let result = client
                .download_attachment(&attachment.content, move |downloaded, total| {
                    let mut s = state_clone.lock().unwrap();
                    *s = FileState::Downloading { downloaded, total };
                    ctx_clone.request_repaint();
                })
                .await;

            match result {
                Ok(data) => {
                    let storage = StorageManager::new(config.download_dir.clone());
                    match storage.save_attachment(&issue_key, &attachment, &data) {
                        Ok(_) => {
                            let mut s = state.lock().unwrap();
                            *s = FileState::Done;
                        }
                        Err(e) => {
                            let mut s = state.lock().unwrap();
                            *s = FileState::Error(e);
                        }
                    }
                }
                Err(e) => {
                    let mut s = state.lock().unwrap();
                    *s = FileState::Error(e);
                }
            }
            ctx.request_repaint();
        });
    }

    pub fn start_all_downloads(
        &self,
        items: &[DownloadItem],
        issue_key: &str,
        config: &AppConfig,
        ctx: egui::Context,
    ) {
        for item in items {
            if item.selected {
                let state = item.current_state();
                if matches!(state, FileState::Pending | FileState::Error(_) | FileState::Done) {
                    self.start_download(item, issue_key, config, ctx.clone());
                }
            }
        }
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
}
