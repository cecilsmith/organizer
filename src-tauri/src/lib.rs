mod app;
mod categorize;
mod commands;
mod db;
mod error;
mod github;
mod gitlab;
mod model;
mod reminders;
mod secrets;

use std::sync::Arc;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_target(false).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = db::Db::open(&dir.join("organizer.db"))?;

            let (changes_tx, changes_rx) = tokio::sync::mpsc::unbounded_channel();
            let reminders = reminders::Reminders::start(move || {
                let _ = changes_tx.send(());
            });

            let state = Arc::new(app::App::new(app.handle().clone(), db, reminders));
            app.manage(state.clone());
            tauri::async_runtime::spawn(state.run_background(changes_rx));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::sync_now,
            commands::add_account,
            commands::import_gh_accounts,
            commands::remove_account,
            commands::repo_candidates,
            commands::project_candidates,
            commands::lookup_repo,
            commands::lookup_project,
            commands::add_repo,
            commands::add_project,
            commands::remove_source,
            commands::set_default_category,
            commands::request_reminders_access,
            commands::request_calendar_access,
            commands::create_event,
            commands::update_event,
            commands::delete_event,
            commands::open_event,
            commands::set_list_enabled,
            commands::upsert_category,
            commands::delete_category,
            commands::create_item,
            commands::set_completed,
            commands::discard_item,
            commands::set_dates,
            commands::set_category,
            commands::item_details,
            commands::update_item,
            commands::open_item,
            commands::open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
