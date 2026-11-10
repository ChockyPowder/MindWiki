//! mindwiki — a personal desktop wiki. Pages are Markdown files in a folder.
//!
//! Usage: `mindwiki [WIKI_FOLDER]` (defaults to the folder in the config, `~/wiki`).
//! The app can also connect to a `mindwiki-server` and sync a shared wiki.

mod app;
mod config;
mod index;
mod page;
mod remote;
mod search;
mod store;
mod sync;
mod ui;

fn main() -> eframe::Result {
    let mut config = config::Config::load();
    if let Some(dir) = std::env::args_os().nth(1) {
        let dir = std::path::PathBuf::from(dir);
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        // Opening a folder by hand means "use this local wiki".
        if dir != config.wiki_root || config.remote.is_some() {
            config.wiki_root = dir;
            config.last_page = None;
            config.remote = None;
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mindwiki")
            .with_app_id("mindwiki")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([500.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        "mindwiki",
        options,
        Box::new(|cc| Ok(Box::new(app::WikiApp::new(&cc.egui_ctx, config)))),
    )
}
