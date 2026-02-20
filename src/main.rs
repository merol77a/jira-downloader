mod app;
mod config;
mod downloader;
mod jira;
mod storage;

use std::sync::Arc;

fn main() -> eframe::Result<()> {
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime"),
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("JIRA Attachment Downloader")
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([700.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "JIRA Attachment Downloader",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new(cc, Arc::clone(&rt))))
        }),
    )
}
