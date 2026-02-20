use std::sync::{Arc, Mutex};

use egui::{Color32, RichText};

use crate::config::AppConfig;
use crate::downloader::{format_size, DownloadItem, DownloadManager, FileState};
use crate::jira::{parse_issue_key, IssueInfo, IssueSummary, JiraClient};
use crate::storage::{ControlFile, IncidentFolder, StorageManager};

#[derive(Debug, Clone, PartialEq)]
enum Tab {
    Settings,
    Incident,
    IncidentsManager,
}

pub struct App {
    runtime: Arc<tokio::runtime::Runtime>,
    tab: Tab,

    // Settings tab
    config: AppConfig,
    config_saved_msg: Option<String>,
    connection_status: Arc<Mutex<Option<Result<String, String>>>>,

    // Incident tab
    incident_input: String,
    fetch_status: Arc<Mutex<Option<Result<IssueInfo, String>>>>,
    current_issue: Option<IssueInfo>,
    download_items: Vec<DownloadItem>,
    download_manager: DownloadManager,

    // My Cases panel
    my_issues: Vec<IssueSummary>,
    my_issues_status: Arc<Mutex<Option<Result<Vec<IssueSummary>, String>>>>,
    my_issues_loading: bool,
    my_issues_error: Option<String>,

    // Incidents Manager tab
    incidents: Vec<IncidentFolder>,
    incidents_scan_status: String,
    check_status: Arc<Mutex<Vec<(String, Result<String, String>)>>>,
    delete_confirm: Option<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, runtime: Arc<tokio::runtime::Runtime>) -> Self {
        let config = AppConfig::load();
        let dm = DownloadManager::new(Arc::clone(&runtime));
        let start_tab = if config.jira_url.is_empty() {
            Tab::Settings
        } else {
            Tab::Incident
        };

        let my_issues_status = Arc::new(Mutex::new(None));

        let mut app = Self {
            runtime,
            tab: start_tab,
            config,
            config_saved_msg: None,
            connection_status: Arc::new(Mutex::new(None)),
            incident_input: String::new(),
            fetch_status: Arc::new(Mutex::new(None)),
            current_issue: None,
            download_items: Vec::new(),
            download_manager: dm,
            my_issues: Vec::new(),
            my_issues_status,
            my_issues_loading: false,
            my_issues_error: None,
            incidents: Vec::new(),
            incidents_scan_status: String::new(),
            check_status: Arc::new(Mutex::new(Vec::new())),
            delete_confirm: None,
        };

        // Auto-load my issues if credentials are already saved
        if !app.config.jira_url.is_empty() && !app.config.email.is_empty() {
            // We can't pass ctx here, loading will trigger on first Incident tab render
            app.my_issues_loading = false; // will be triggered in render
        }

        app
    }

    // ─── Settings ──────────────────────────────────────────────────────────────

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.add_space(8.0);

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("JIRA URL:");
                ui.text_edit_singleline(&mut self.config.jira_url);
                ui.end_row();

                ui.label("Email:");
                ui.text_edit_singleline(&mut self.config.email);
                ui.end_row();

                ui.label("API Token:");
                ui.add(egui::TextEdit::singleline(&mut self.config.api_token).password(true));
                ui.end_row();

                ui.label("Download Directory:");
                ui.horizontal(|ui| {
                    ui.label(self.config.download_dir.to_string_lossy().as_ref());
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.config.download_dir = path;
                        }
                    }
                });
                ui.end_row();
            });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(RichText::new("How to get an API Token").strong());
        ui.add_space(4.0);
        egui::Grid::new("api_help_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(RichText::new("1.").strong());
                ui.label("Log in to Atlassian with your company account.");
                ui.end_row();

                ui.label(RichText::new("2.").strong());
                ui.horizontal(|ui| {
                    ui.label("Open your API token page:");
                    ui.hyperlink_to(
                        "id.atlassian.com → Security → API tokens",
                        "https://id.atlassian.com/manage-profile/security/api-tokens",
                    );
                });
                ui.end_row();

                ui.label(RichText::new("3.").strong());
                ui.label("Click \"Create API token\", give it a name, copy the token.");
                ui.end_row();

                ui.label(RichText::new("4.").strong());
                ui.label("Paste it in the API Token field above and click Save.");
                ui.end_row();

                ui.label(RichText::new("Note:").strong());
                ui.colored_label(
                    Color32::from_rgb(200, 120, 0),
                    "If your company uses SSO, generate the token from a mobile hotspot \
                     or ask IT to whitelist id.atlassian.com.",
                );
                ui.end_row();
            });

        ui.add_space(12.0);

        // Buttons — capture clicks as booleans, apply actions after closures
        let (save_clicked, test_clicked) = ui
            .horizontal(|ui| (ui.button("Save").clicked(), ui.button("Test Connection").clicked()))
            .inner;

        if save_clicked {
            match self.config.save() {
                Ok(_) => self.config_saved_msg = Some("Configuration saved.".to_string()),
                Err(e) => self.config_saved_msg = Some(format!("Error: {e}")),
            }
        }

        if test_clicked {
            *self.connection_status.lock().unwrap() = None;
            let config = self.config.clone();
            let status = Arc::clone(&self.connection_status);
            let ctx_clone = ui.ctx().clone();
            self.runtime.spawn(async move {
                let client = JiraClient::new(config);
                let result = client.test_connection().await;
                *status.lock().unwrap() = Some(result);
                ctx_clone.request_repaint();
            });
        }

        if let Some(msg) = &self.config_saved_msg {
            ui.colored_label(Color32::GREEN, msg);
        }

        let conn_status = self.connection_status.lock().unwrap().clone();
        match conn_status {
            Some(Ok(msg)) => {
                ui.colored_label(Color32::GREEN, format!("✓ {msg}"));
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::RED, format!("✗ {e}"));
            }
            None => {}
        }
    }

    fn load_my_issues(&mut self, ctx: &egui::Context) {
        if self.my_issues_loading { return; }
        self.my_issues_loading = true;
        *self.my_issues_status.lock().unwrap() = None;

        let config = self.config.clone();
        let status = Arc::clone(&self.my_issues_status);
        let ctx = ctx.clone();

        self.runtime.spawn(async move {
            let client = JiraClient::new(config);
            let result = client.fetch_my_issues().await;
            *status.lock().unwrap() = Some(result);
            ctx.request_repaint();
        });
    }

    // ─── Incident ──────────────────────────────────────────────────────────────

    fn render_incident(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Auto-load my issues on first render if credentials are present and no error yet
        if !self.my_issues_loading
            && self.my_issues.is_empty()
            && self.my_issues_error.is_none()
            && !self.config.jira_url.is_empty()
            && !self.config.email.is_empty()
        {
            self.load_my_issues(ctx);
        }

        // Process incoming my-issues result
        let my_result = self.my_issues_status.lock().unwrap().take();
        match my_result {
            Some(Ok(issues)) => {
                self.my_issues = issues;
                self.my_issues_loading = false;
                self.my_issues_error = None;
            }
            Some(Err(e)) => {
                self.my_issues_loading = false;
                self.my_issues_error = Some(e);
            }
            None => {}
        }

        // ── Incident input row (top) ──────────────────────────────────────────
        let fetch_triggered = ui
            .horizontal(|ui| {
                ui.label(RichText::new("Incident:").strong());
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.incident_input)
                        .hint_text("PROJ-123 or full JIRA URL")
                        .desired_width(300.0),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.button("Fetch").clicked() || enter
            })
            .inner;

        if fetch_triggered {
            self.do_fetch(ctx);
        }

        ui.add_space(4.0);

        // ── My Cases panel ────────────────────────────────────────────────────
        let mut selected_key: Option<String> = None;

        egui::CollapsingHeader::new(
            RichText::new(format!("My Open Cases ({})", self.my_issues.len())).strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            if self.my_issues_loading && self.my_issues.is_empty() {
                ui.colored_label(Color32::GRAY, "Loading...");
            } else if let Some(err) = &self.my_issues_error {
                ui.colored_label(Color32::from_rgb(200, 60, 60), format!("Error: {err}"));
                if ui.small_button("↻ Retry").clicked() {
                    self.my_issues_error = None;
                    self.my_issues_loading = false;
                    self.load_my_issues(ctx);
                }
            } else if self.my_issues.is_empty() {
                ui.colored_label(Color32::GRAY, "No open cases assigned to you.");
            } else {
                let refresh = ui.small_button("↻ Refresh").clicked();
                if refresh {
                    self.my_issues_loading = false;
                    self.load_my_issues(ctx);
                }

                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("my_cases_scroll")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        egui::Grid::new("my_cases_grid")
                            .num_columns(3)
                            .spacing([12.0, 4.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for issue in &self.my_issues {
                                    let is_current = self
                                        .current_issue
                                        .as_ref()
                                        .map(|c| c.key == issue.key)
                                        .unwrap_or(false);

                                    let key_text = if is_current {
                                        RichText::new(&issue.key).strong().color(Color32::from_rgb(80, 160, 240))
                                    } else {
                                        RichText::new(&issue.key).strong()
                                    };

                                    if ui.button(key_text).clicked() {
                                        selected_key = Some(issue.key.clone());
                                    }

                                    let summary = if issue.summary.len() > 50 {
                                        format!("{}...", &issue.summary[..50])
                                    } else {
                                        issue.summary.clone()
                                    };
                                    ui.label(summary);

                                    let sc = status_color(&issue.status);
                                    ui.colored_label(sc, &issue.status);
                                    ui.end_row();
                                }
                            });
                    });
            }
        });

        if let Some(key) = selected_key {
            self.incident_input = key;
            self.do_fetch(ctx);
        }

        ui.separator();
        ui.add_space(4.0);

        // Process fetch result — update self before any rendering borrows
        let fetch_result = self.fetch_status.lock().unwrap().clone();
        match fetch_result {
            Some(Ok(issue)) => {
                let storage = StorageManager::new(self.config.download_dir.clone());
                let ctrl = ControlFile::new(&issue.key, &issue.summary, &issue.status);
                let _ = storage.save_control_file(&ctrl);
                self.download_items = issue
                    .attachments
                    .iter()
                    .map(|a| {
                        let on_disk = storage.attachment_exists(&issue.key, a);
                        let mut item = DownloadItem::new(a.clone());
                        if on_disk {
                            item.selected = false;
                            *item.state.lock().unwrap() = FileState::AlreadyOnDisk;
                        }
                        item
                    })
                    .collect();
                self.current_issue = Some(issue);
                *self.fetch_status.lock().unwrap() = None;
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::RED, format!("Error: {e}"));
            }
            None => {}
        }

        // Extract display data as owned values so no borrow on self.current_issue remains
        let issue_data: Option<(String, String, String)> = self
            .current_issue
            .as_ref()
            .map(|i| (i.key.clone(), i.summary.clone(), i.status.clone()));

        if let Some((issue_key, summary, status)) = issue_data {
            let open_folder = ui
                .horizontal(|ui| {
                    ui.label(RichText::new(&issue_key).strong());
                    ui.label("—");
                    ui.label(&summary);
                    ui.label("|");
                    ui.label(RichText::new(&status).italics());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.button("📁 Open Folder").clicked()
                    })
                    .inner
                })
                .inner;

            if open_folder {
                let storage = StorageManager::new(self.config.download_dir.clone());
                let path = storage.latest_date_folder(&issue_key);
                StorageManager::open_path(&path);
            }

            ui.separator();

            let count = self.download_items.len();
            ui.label(format!("Attachments ({count}):"));
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    egui::Grid::new("attachments_grid")
                        .num_columns(6)
                        .spacing([8.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for item in &mut self.download_items {
                                let state = item.current_state();
                                ui.checkbox(&mut item.selected, "");
                                ui.label(&item.attachment.filename);
                                ui.label(format_size(item.attachment.size));
                                ui.label(
                                    item.attachment.created.format("%Y-%m-%d").to_string(),
                                );
                                let frac = state.progress_fraction().unwrap_or(0.0);
                                ui.add(
                                    egui::ProgressBar::new(frac)
                                        .desired_width(120.0)
                                        .show_percentage(),
                                );
                                let label = state.label();
                                match &state {
                                    FileState::Done | FileState::AlreadyOnDisk => {
                                        ui.colored_label(Color32::from_rgb(60, 180, 60), &label);
                                    }
                                    FileState::Error(_) => {
                                        ui.colored_label(Color32::from_rgb(200, 60, 60), &label);
                                    }
                                    _ => {
                                        ui.label(&label);
                                    }
                                };
                                ui.end_row();
                            }
                        });
                });

            ui.add_space(8.0);

            // All action buttons in one row: Download Selected | Download All | Select All | Deselect All
            let (dl_selected, dl_all, select_all, deselect_all) = ui
                .horizontal(|ui| {
                    let ds = ui.button("Download Selected").clicked();
                    let da = ui.button("Download All").clicked();
                    ui.add_space(8.0);
                    let sa = ui.button("Select All").clicked();
                    let de = ui.button("Deselect All").clicked();
                    (ds, da, sa, de)
                })
                .inner;

            if select_all {
                for item in &mut self.download_items {
                    item.selected = true;
                }
            }
            if deselect_all {
                for item in &mut self.download_items {
                    item.selected = false;
                }
            }
            if dl_all {
                for item in &mut self.download_items {
                    item.selected = true;
                }
            }
            if dl_selected || dl_all {
                self.download_manager.start_all_downloads(
                    &self.download_items,
                    &issue_key,
                    &self.config,
                    ctx.clone(),
                );
            }
        } else {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Enter an issue key or URL above and click Fetch")
                        .color(Color32::GRAY),
                );
            });
        }
    }

    fn do_fetch(&mut self, ctx: &egui::Context) {
        let input = self.incident_input.trim().to_string();
        let key = match parse_issue_key(&input) {
            Some(k) => k,
            None => {
                *self.fetch_status.lock().unwrap() =
                    Some(Err("Invalid issue key or URL".to_string()));
                return;
            }
        };

        self.current_issue = None;
        self.download_items.clear();
        *self.fetch_status.lock().unwrap() = None;

        let config = self.config.clone();
        let status = Arc::clone(&self.fetch_status);
        let ctx = ctx.clone();

        self.runtime.spawn(async move {
            let client = JiraClient::new(config);
            let result = client.fetch_issue(&key).await;
            *status.lock().unwrap() = Some(result);
            ctx.request_repaint();
        });
    }

    // ─── Incidents Manager ─────────────────────────────────────────────────────

    fn render_incidents_manager(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Incidents Manager");
        ui.add_space(8.0);

        // 1. Process any pending async status updates before rendering
        let updates: Vec<(String, Result<String, String>)> = {
            self.check_status.lock().unwrap().drain(..).collect()
        };
        for (key, result) in updates {
            if let Some(incident) = self
                .incidents
                .iter_mut()
                .find(|i| i.control.issue_key == key)
            {
                match result {
                    Ok(status) => {
                        incident.control.issue_status = status;
                        incident.control.last_checked = chrono::Utc::now();
                        incident.control.marked_for_deletion = incident.control.is_closed();
                        let storage = StorageManager::new(self.config.download_dir.clone());
                        let _ = storage.save_control_file(&incident.control);
                    }
                    Err(e) => {
                        self.incidents_scan_status = format!("Error checking {key}: {e}");
                    }
                }
            }
        }

        // 2. Header buttons — extract click results before touching self
        let (scan_clicked, check_all_clicked, delete_all_clicked) = ui
            .horizontal(|ui| {
                (
                    ui.button("Scan Folder").clicked(),
                    ui.button("Check All Status").clicked(),
                    ui.button("Delete All Marked").clicked(),
                )
            })
            .inner;

        if scan_clicked {
            let storage = StorageManager::new(self.config.download_dir.clone());
            self.incidents = storage.scan_incidents();
            self.incidents_scan_status =
                format!("Found {} incident(s).", self.incidents.len());
        }
        if check_all_clicked {
            self.check_all_statuses(ctx);
        }
        if delete_all_clicked {
            self.delete_all_marked();
        }

        if !self.incidents_scan_status.is_empty() {
            ui.label(&self.incidents_scan_status.clone());
        }

        ui.add_space(8.0);
        ui.separator();

        if self.incidents.is_empty() {
            ui.colored_label(Color32::GRAY, "No incidents found. Click 'Scan Folder'.");
        } else {
            // 3. Render grid — collect action intents, don't mutate self inside closures
            let mut to_check: Option<String> = None;
            let mut to_open: Option<String> = None;
            let mut to_delete: Option<String> = None;

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("incidents_grid")
                    .num_columns(7)
                    .spacing([8.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Issue").strong());
                        ui.label(RichText::new("Summary").strong());
                        ui.label(RichText::new("Status").strong());
                        ui.label(RichText::new("Size").strong());
                        ui.label(RichText::new("Last Checked").strong());
                        ui.label(RichText::new("Actions").strong());
                        ui.label("");
                        ui.end_row();

                        for incident in &self.incidents {
                            let ctrl = &incident.control;
                            let is_closed = ctrl.is_closed();
                            let key = ctrl.issue_key.clone();

                            ui.label(RichText::new(&key).strong());

                            let summary = if ctrl.issue_summary.len() > 28 {
                                format!("{}...", &ctrl.issue_summary[..28])
                            } else {
                                ctrl.issue_summary.clone()
                            };
                            ui.label(summary);

                            let status_color =
                                if is_closed { Color32::from_rgb(200, 60, 60) } else { Color32::from_rgb(60, 180, 60) };
                            ui.colored_label(status_color, &ctrl.issue_status);

                            ui.label(format_size(incident.folder_size));

                            let elapsed = chrono::Utc::now()
                                .signed_duration_since(ctrl.last_checked);
                            ui.label(format_duration(elapsed));

                            ui.horizontal(|ui| {
                                if ui.button("Check").clicked() {
                                    to_check = Some(key.clone());
                                }
                                if ui.button("Open").clicked() {
                                    to_open = Some(key.clone());
                                }
                            });

                            if is_closed || ctrl.marked_for_deletion {
                                if ui
                                    .button(
                                        RichText::new("Delete ⚠").color(Color32::RED),
                                    )
                                    .clicked()
                                {
                                    to_delete = Some(key.clone());
                                }
                            } else {
                                ui.label("");
                            }

                            ui.end_row();
                        }
                    });
            });

            // 4. Apply collected actions (self is free again)
            if let Some(key) = to_check {
                self.check_single_status(&key, ctx);
            }
            if let Some(key) = to_open {
                let storage = StorageManager::new(self.config.download_dir.clone());
                storage.open_folder(&key);
            }
            if let Some(key) = to_delete {
                self.delete_confirm = Some(key);
            }
        }

        // 5. Deletion confirmation dialog
        if let Some(key) = self.delete_confirm.clone() {
            let mut confirmed = false;
            let mut cancelled = false;

            egui::Window::new("Confirm Deletion")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Delete folder for {key}? This cannot be undone."
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Yes, Delete").clicked() {
                            confirmed = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancelled = true;
                        }
                    });
                });

            if confirmed {
                let storage = StorageManager::new(self.config.download_dir.clone());
                match storage.delete_folder(&key) {
                    Ok(_) => {
                        self.incidents.retain(|i| i.control.issue_key != key);
                        self.incidents_scan_status = format!("Deleted folder for {key}.");
                    }
                    Err(e) => {
                        self.incidents_scan_status = format!("Delete failed: {e}");
                    }
                }
                self.delete_confirm = None;
            } else if cancelled {
                self.delete_confirm = None;
            }
        }
    }

    fn check_single_status(&self, issue_key: &str, ctx: &egui::Context) {
        let config = self.config.clone();
        let key = issue_key.to_string();
        let updates = Arc::clone(&self.check_status);
        let ctx = ctx.clone();

        self.runtime.spawn(async move {
            let client = JiraClient::new(config);
            let result = client.fetch_issue_status(&key).await;
            updates.lock().unwrap().push((key, result));
            ctx.request_repaint();
        });
    }

    fn check_all_statuses(&self, ctx: &egui::Context) {
        for incident in &self.incidents {
            self.check_single_status(&incident.control.issue_key, ctx);
        }
    }

    fn delete_all_marked(&mut self) {
        let storage = StorageManager::new(self.config.download_dir.clone());
        let keys: Vec<String> = self
            .incidents
            .iter()
            .filter(|i| i.control.marked_for_deletion || i.control.is_closed())
            .map(|i| i.control.issue_key.clone())
            .collect();

        let mut deleted = 0;
        let mut errors: Vec<String> = Vec::new();

        for key in &keys {
            match storage.delete_folder(key) {
                Ok(_) => deleted += 1,
                Err(e) => errors.push(format!("{key}: {e}")),
            }
        }

        self.incidents.retain(|i| !keys.contains(&i.control.issue_key));

        self.incidents_scan_status = if errors.is_empty() {
            format!("Deleted {deleted} folder(s).")
        } else {
            format!("Deleted {deleted}, errors: {}", errors.join("; "))
        };
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Incident, "Incident");
                ui.selectable_value(
                    &mut self.tab,
                    Tab::IncidentsManager,
                    "Incidents Manager",
                );
                ui.selectable_value(&mut self.tab, Tab::Settings, "⚙ Settings");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.tab.clone() {
                Tab::Settings => self.render_settings(ui),
                Tab::Incident => self.render_incident(ui, ctx),
                Tab::IncidentsManager => self.render_incidents_manager(ui, ctx),
            }
        });
    }
}

fn status_color(status: &str) -> Color32 {
    let s = status.to_lowercase();
    if s.contains("progress") || s.contains("review") || s.contains("open") {
        Color32::from_rgb(80, 160, 240)
    } else if s.contains("done") || s.contains("closed") || s.contains("resolv") {
        Color32::from_rgb(120, 130, 145)
    } else {
        Color32::from_gray(170)
    }
}

fn format_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds().unsigned_abs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
