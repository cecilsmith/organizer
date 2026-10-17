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

use tauri::{Manager, PhysicalSize, WebviewWindow};

/// A restored window has to keep at least this much of itself on a display,
/// otherwise it's recentered.
const MIN_VISIBLE: f64 = 0.4;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_target(false).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Restores the window's size and position from the last run.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                if let Err(e) = keep_on_screen(&win) {
                    tracing::warn!("couldn't check the window's position: {e}");
                }
            }
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
            commands::import_glab_accounts,
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

/// Makes sure a window restored from the last run is still usable on today's
/// displays: shrinks it to fit the screen it opened on, and recenters it when
/// too little of it is visible or its title bar sits above the work area (under
/// the menu bar), where it couldn't be dragged back.
fn keep_on_screen(win: &WebviewWindow) -> tauri::Result<()> {
    let areas: Vec<Rect> = win
        .available_monitors()?
        .iter()
        .map(|m| {
            let a = m.work_area();
            ((a.position.x, a.position.y), (a.size.width, a.size.height))
        })
        .collect();
    let pos = win.outer_position()?;
    let size = win.outer_size()?;
    let ((width, height), recenter) = fit((pos.x, pos.y), (size.width, size.height), &areas);
    if (width, height) != (size.width, size.height) {
        win.set_size(PhysicalSize::new(width, height))?;
    }
    if recenter {
        win.center()?;
    }
    Ok(())
}

/// `((x, y), (width, height))` in physical pixels.
type Rect = ((i32, i32), (u32, u32));

/// The size a window should take and whether it needs recentering, given where
/// it was restored and the work areas of the displays that exist now.
fn fit(pos: (i32, i32), size: (u32, u32), areas: &[Rect]) -> ((u32, u32), bool) {
    // The display the window landed on, by how much of it is there. With no
    // overlap anywhere — a display that's since been unplugged — any of them
    // will do, since the window gets recentered below either way.
    let Some(home) = areas.iter().max_by_key(|a| overlap((pos, size), **a)) else {
        return (size, false);
    };
    let size = (size.0.min(home.1.0), size.1.min(home.1.1));
    let visible: i64 = areas.iter().map(|a| overlap((pos, size), *a)).sum();
    let area = i64::from(size.0) * i64::from(size.1);
    // Above the work area the title bar is under the menu bar, out of reach.
    let title_bar_hidden = pos.1 < home.0.1;
    (size, title_bar_hidden || (visible as f64) < area as f64 * MIN_VISIBLE)
}

/// Area shared by two rectangles, in square pixels.
fn overlap(a: Rect, b: Rect) -> i64 {
    let side = |(ap, aw): (i32, u32), (bp, bw): (i32, u32)| {
        i64::from(((ap + aw as i32).min(bp + bw as i32) - ap.max(bp)).max(0))
    };
    side((a.0.0, a.1.0), (b.0.0, b.1.0)) * side((a.0.1, a.1.1), (b.0.1, b.1.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Rect = ((0, 38), (1512, 944));
    /// A second display to the left of the laptop, as macOS lays them out.
    const EXTERNAL: Rect = ((-2560, 0), (2560, 1440));

    #[test]
    fn leaves_a_window_on_its_display_alone() {
        assert_eq!(fit((120, 60), (1180, 780), &[LAPTOP]), ((1180, 780), false));
    }

    #[test]
    fn recenters_a_window_from_an_unplugged_display() {
        let (size, recenter) = fit((-1800, 400), (1180, 780), &[LAPTOP]);
        assert_eq!(size, (1180, 780));
        assert!(recenter);
    }

    #[test]
    fn keeps_a_window_that_straddles_two_displays() {
        assert_eq!(fit((-400, 300), (1180, 780), &[LAPTOP, EXTERNAL]), ((1180, 780), false));
    }

    #[test]
    fn recenters_a_window_hanging_off_the_edge() {
        assert!(fit((1400, 300), (1180, 780), &[LAPTOP]).1);
    }

    #[test]
    fn recenters_a_window_whose_title_bar_is_under_the_menu_bar() {
        assert!(fit((100, 0), (1180, 780), &[LAPTOP]).1);
    }

    #[test]
    fn shrinks_a_window_saved_on_a_bigger_display() {
        assert_eq!(fit((0, 38), (2400, 1300), &[LAPTOP]), ((1512, 944), false));
    }

    #[test]
    fn does_nothing_without_any_displays() {
        assert_eq!(fit((0, 0), (1180, 780), &[]), ((1180, 780), false));
    }
}
