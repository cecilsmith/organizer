//! Thin IPC layer: each command forwards to `App`.

use std::sync::Arc;

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::app::App;
use crate::error::Result;
use crate::model::*;

type S<'a> = State<'a, Arc<App>>;

#[tauri::command]
pub fn get_snapshot(app: S<'_>) -> Result<Snapshot> {
    app.snapshot()
}

#[tauri::command]
pub async fn sync_now(app: S<'_>) -> Result<()> {
    let app = Arc::clone(&app);
    tokio::spawn(async move { app.sync_all().await });
    Ok(())
}

#[tauri::command]
pub async fn add_account(app: S<'_>, token: String, host: Option<String>) -> Result<Account> {
    let account = app.add_account(&token, host.as_deref().unwrap_or("github.com")).await?;
    Ok(account)
}

#[tauri::command]
pub async fn import_gh_accounts(app: S<'_>) -> Result<Vec<String>> {
    app.import_gh_accounts().await
}

#[tauri::command]
pub fn remove_account(app: S<'_>, id: String) -> Result<()> {
    app.remove_account(&id)
}

#[tauri::command]
pub async fn repo_candidates(app: S<'_>, account_id: String) -> Result<Vec<Candidate>> {
    app.repo_candidates(&account_id).await
}

#[tauri::command]
pub async fn project_candidates(app: S<'_>, account_id: String) -> Result<Vec<Candidate>> {
    app.project_candidates(&account_id).await
}

#[tauri::command]
pub async fn lookup_repo(app: S<'_>, account_id: String, spec: String) -> Result<Candidate> {
    app.lookup_repo(&account_id, &spec).await
}

#[tauri::command]
pub async fn lookup_project(app: S<'_>, account_id: String, spec: String) -> Result<Candidate> {
    app.lookup_project(&account_id, &spec).await
}

#[tauri::command]
pub async fn add_repo(app: S<'_>, account_id: String, candidate: Candidate, category: Option<String>) -> Result<()> {
    app.add_repo(&account_id, candidate, category).await
}

#[tauri::command]
pub async fn add_project(
    app: S<'_>,
    account_id: String,
    candidate: Candidate,
    category: Option<String>,
) -> Result<()> {
    app.add_project(&account_id, candidate, category).await
}

#[tauri::command]
pub fn remove_source(app: S<'_>, id: String) -> Result<()> {
    app.remove_source(&id)
}

#[tauri::command]
pub fn set_default_category(app: S<'_>, container_id: String, category: Option<String>) -> Result<()> {
    app.set_default_category(&container_id, category.as_deref())
}

#[tauri::command]
pub async fn request_reminders_access(app: S<'_>) -> Result<String> {
    app.request_reminders_access().await
}

#[tauri::command]
pub async fn set_list_enabled(app: S<'_>, id: String, enabled: bool) -> Result<()> {
    app.set_list_enabled(&id, enabled).await
}

#[tauri::command]
pub fn upsert_category(app: S<'_>, category: Category) -> Result<Category> {
    app.upsert_category(category)
}

#[tauri::command]
pub fn delete_category(app: S<'_>, id: String) -> Result<()> {
    app.delete_category(&id)
}

#[tauri::command]
pub async fn create_item(app: S<'_>, item: NewItem) -> Result<Item> {
    app.create_item(item).await
}

#[tauri::command]
pub async fn set_completed(app: S<'_>, id: String, completed: bool) -> Result<()> {
    app.set_completed(&id, completed).await
}

#[tauri::command]
pub async fn set_category(app: S<'_>, id: String, category: Option<String>) -> Result<()> {
    app.set_category(&id, category).await
}

#[tauri::command]
pub async fn item_details(app: S<'_>, id: String) -> Result<ItemDetails> {
    app.item_details(&id).await
}

#[tauri::command]
pub async fn update_item(app: S<'_>, id: String, patch: ItemPatch) -> Result<Item> {
    app.update_item(&id, patch).await
}

#[tauri::command]
pub fn open_item(handle: tauri::AppHandle, app: S<'_>, id: String) -> Result<()> {
    let url = app.item_url(&id)?;
    open(&handle, &url)
}

#[tauri::command]
pub fn open_url(handle: tauri::AppHandle, url: String) -> Result<()> {
    open(&handle, &url)
}

fn open(handle: &tauri::AppHandle, url: &str) -> Result<()> {
    let allowed = ["https://", "x-apple-reminderkit://", "x-apple.systempreferences:"];
    if !allowed.iter().any(|p| url.starts_with(p)) {
        return Err(crate::error::Error::msg("refusing to open that link"));
    }
    handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| crate::error::Error::msg(e.to_string()))
}
