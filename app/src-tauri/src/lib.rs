pub mod orchestration;
pub use db::Database as LocalDatabase;
pub mod capability_identity;
mod codex_scanner;
mod db;
mod github;
mod installer;
pub mod intelligence;
pub mod learning;
mod lifecycle;
pub mod routing;

use db::{
    CapabilityRecord, Database, DomainRecord, GraphLayoutInput, GraphPosition, InstallationRecord,
    LearningInput, LearningRecord, RelationRecord,
};
use std::{path::PathBuf, sync::Arc};
use tauri::Manager;

pub struct DatabaseState {
    database: Option<Database>,
    new_profile: bool,
}

pub struct GitHubState(Option<Arc<github::GitHubDiscoveryService>>);
impl GitHubState {
    fn service(&self) -> Result<&github::GitHubDiscoveryService, github::types::ApiError> {
        self.0
            .as_deref()
            .ok_or_else(github::types::ApiError::database)
    }
}
pub struct InstallerState(Option<Arc<installer::Installer>>);
impl InstallerState {
    fn installer(&self) -> Result<&installer::Installer, github::types::ApiError> {
        self.0.as_deref().ok_or_else(|| {
            github::types::ApiError::new(
                "INSTALLER_UNAVAILABLE",
                "The native installer is unavailable; no files were changed.",
            )
        })
    }
}
#[tauri::command]
async fn workflow_engine(
    action: String,
    args: serde_json::Value,
    database: tauri::State<'_, DatabaseState>,
) -> Result<serde_json::Value, String> {
    let mut result = database.database()?.orchestration(&action, args).await?;
    result["isolated"] = serde_json::json!(isolated_installation_root().is_some());
    Ok(result)
}

#[tauri::command]
async fn learning_engine(
    action: String,
    args: serde_json::Value,
    database: tauri::State<'_, DatabaseState>,
) -> Result<serde_json::Value, String> {
    let mut result = database.database()?.learning_engine(&action, args).await?;
    result["isolated"] = serde_json::json!(isolated_installation_root().is_some());
    Ok(result)
}
#[tauri::command]
async fn capability_intelligence(
    action: String,
    args: serde_json::Value,
    database: tauri::State<'_, DatabaseState>,
) -> Result<serde_json::Value, String> {
    let mut value = database.database()?.intelligence(&action, args).await?;
    if action == "state" {
        value["isolated"] = serde_json::json!(isolated_installation_root().is_some());
    }
    Ok(value)
}

#[tauri::command]
async fn intelligence_credential(
    value: Option<String>,
    database: tauri::State<'_, DatabaseState>,
) -> Result<(), String> {
    database
        .database()?
        .intelligence("credential", serde_json::json!({"value":value}))
        .await?;
    Ok(())
}

#[tauri::command]
async fn installation_state(
    state: tauri::State<'_, InstallerState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.installer()?.state().await
}
#[tauri::command]
async fn installation_operation(
    operation_id: String,
    state: tauri::State<'_, InstallerState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.installer()?.operation(&operation_id).await
}
#[tauri::command]
async fn installation_prepare(
    request: installer::types::Request,
    state: tauri::State<'_, InstallerState>,
) -> Result<installer::types::Operation, github::types::ApiError> {
    state.installer()?.prepare(request).await
}
#[tauri::command]
async fn installation_execute(
    operation_id: String,
    plan_digest: String,
    confirmed: bool,
    state: tauri::State<'_, InstallerState>,
) -> Result<installer::types::Operation, github::types::ApiError> {
    state
        .installer()?
        .execute(&operation_id, &plan_digest, confirmed)
        .await
}
#[tauri::command]
async fn installation_cancel(
    operation_id: String,
    state: tauri::State<'_, InstallerState>,
) -> Result<(), github::types::ApiError> {
    state.installer()?.cancel(&operation_id).await
}
#[tauri::command]
async fn installation_recover(
    operation_id: String,
    plan_digest: String,
    confirmed: bool,
    state: tauri::State<'_, InstallerState>,
) -> Result<installer::types::Operation, github::types::ApiError> {
    state
        .installer()?
        .recover(&operation_id, &plan_digest, confirmed)
        .await
}
#[tauri::command]
async fn installation_verify(
    entity_id: String,
    state: tauri::State<'_, InstallerState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.installer()?.verify(&entity_id).await
}
pub(crate) fn isolated_installation_root() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("SKILLVERSE_ISOLATION_ROOT").map(PathBuf::from)
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}
#[tauri::command]
async fn github_discovery_state(
    state: tauri::State<'_, GitHubState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.service()?.state().await
}
#[tauri::command]
async fn github_run(
    request_id: String,
    operation: github::types::Operation,
    state: tauri::State<'_, GitHubState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.service()?.execute(&request_id, operation).await
}
#[tauri::command]
async fn github_cancel(
    request_id: String,
    state: tauri::State<'_, GitHubState>,
) -> Result<(), github::types::ApiError> {
    state.service()?.cancel(&request_id)
}
#[tauri::command]
async fn github_resume(
    run_id: String,
    request_id: String,
    state: tauri::State<'_, GitHubState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.service()?.resume(&run_id, &request_id).await
}
#[tauri::command]
async fn github_set_token(
    token: Option<String>,
    state: tauri::State<'_, GitHubState>,
) -> Result<(), github::types::ApiError> {
    state.service()?.set_token(token).await
}
#[tauri::command]
async fn github_classify(
    entity_id: String,
    primary: Option<String>,
    related: Vec<String>,
    state: tauri::State<'_, GitHubState>,
) -> Result<(), github::types::ApiError> {
    state
        .service()?
        .classify_manually(&entity_id, primary, related)
        .await
}
#[tauri::command]
async fn github_installation_plan(
    entity_id: String,
    state: tauri::State<'_, GitHubState>,
) -> Result<serde_json::Value, github::types::ApiError> {
    state.service()?.installation_plan(&entity_id).await
}

impl DatabaseState {
    fn database(&self) -> Result<&Database, String> {
        self.database.as_ref().ok_or_else(|| {
            "Local database is unavailable. Persistence is disabled for this session.".to_string()
        })
    }
}

#[tauri::command]
async fn list_capabilities(
    database: tauri::State<'_, DatabaseState>,
) -> Result<Vec<CapabilityRecord>, String> {
    database.database()?.list_capabilities().await
}

#[tauri::command]
async fn get_capability(
    id: String,
    database: tauri::State<'_, DatabaseState>,
) -> Result<Option<CapabilityRecord>, String> {
    database.database()?.get_capability(&id).await
}

#[tauri::command]
async fn list_domains(
    database: tauri::State<'_, DatabaseState>,
) -> Result<Vec<DomainRecord>, String> {
    database.database()?.list_domains().await
}

#[tauri::command]
async fn get_capability_relations(
    id: String,
    database: tauri::State<'_, DatabaseState>,
) -> Result<Vec<RelationRecord>, String> {
    database.database()?.get_capability_relations(&id).await
}

#[tauri::command]
async fn save_graph_layout(
    input: GraphLayoutInput,
    database: tauri::State<'_, DatabaseState>,
) -> Result<(), String> {
    database.database()?.save_graph_layout(input).await
}

#[tauri::command]
async fn load_graph_layout(
    profile: String,
    database: tauri::State<'_, DatabaseState>,
) -> Result<Vec<GraphPosition>, String> {
    database.database()?.load_graph_layout(&profile).await
}

#[tauri::command]
async fn get_installation_status(
    id: String,
    database: tauri::State<'_, DatabaseState>,
) -> Result<Option<InstallationRecord>, String> {
    database.database()?.get_installation_status(&id).await
}

#[tauri::command]
async fn get_learning_progress(
    id: String,
    database: tauri::State<'_, DatabaseState>,
) -> Result<Option<LearningRecord>, String> {
    database.database()?.get_learning_progress(&id).await
}

#[tauri::command]
async fn update_learning_progress(
    input: LearningInput,
    database: tauri::State<'_, DatabaseState>,
) -> Result<LearningRecord, String> {
    database.database()?.update_learning_progress(input).await
}

#[tauri::command]
async fn check_database_integrity(database: tauri::State<'_, DatabaseState>) -> Result<(), String> {
    database.database()?.integrity_check().await
}

#[tauri::command]
fn desktop_startup(database: tauri::State<'_, DatabaseState>) -> serde_json::Value {
    serde_json::json!({
        "newProfile": database.new_profile && database.database.is_some(),
        "codexCliOnPath": codex_scanner::cli_exists_on_path(),
    })
}

#[tauri::command]
async fn create_database_backup(
    database: tauri::State<'_, DatabaseState>,
) -> Result<String, String> {
    database.database()?.create_verified_backup().await
}

#[tauri::command]
async fn scan_codex(
    _database: tauri::State<'_, DatabaseState>,
    installer: tauri::State<'_, InstallerState>,
) -> Result<codex_scanner::CodexScanReport, String> {
    if let Some(service) = installer.0.as_ref() {
        return service.scan().await.map_err(|error| error.message);
    }
    if isolated_installation_root().is_some() {
        return Err(
            "The isolated scanner is unavailable; no real profile fallback is allowed.".into(),
        );
    }
    Err("An explicit selected Codex/project identity context is required; no legacy scanner fallback was persisted.".into())
}

#[tauri::command]
async fn capability_reconciliation_preview(
    backup: capability_identity::BackupRef,
    database: tauri::State<'_, DatabaseState>,
    installer: tauri::State<'_, InstallerState>,
) -> Result<capability_identity::ReconciliationPlan, String> {
    let context = installer
        .installer()
        .map_err(|e| e.message)?
        .identity_context();
    database
        .database()?
        .preview_identity_reconciliation(context, backup)
        .await
}

#[tauri::command]
async fn capability_reconciliation_apply(
    plan: capability_identity::ReconciliationPlan,
    plan_digest: String,
    confirmed: bool,
    database: tauri::State<'_, DatabaseState>,
    installer: tauri::State<'_, InstallerState>,
) -> Result<(), String> {
    if plan.context
        != installer
            .installer()
            .map_err(|e| e.message)?
            .identity_context()
    {
        return Err(
            "The selected real/isolated context changed; prepare a new reconciliation.".into(),
        );
    }
    database
        .database()?
        .apply_identity_reconciliation(&plan, &plan_digest, confirmed)
        .await
}

#[tauri::command]
async fn get_codex_scan(
    database: tauri::State<'_, DatabaseState>,
) -> Result<Option<codex_scanner::CodexScanReport>, String> {
    database.database()?.load_codex_scan().await
}

fn app_database_path(app: &tauri::AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(root) = isolated_installation_root() {
        let profile = installer::types::Profile::isolated(root.clone())
            .map_err(|_| std::io::Error::other("Invalid isolated profile"))?;
        std::fs::create_dir_all(&profile.work_root)?;
        return Ok(root.join("installation.sqlite3"));
    }
    let directory = app.path().app_data_dir()?;
    #[cfg(windows)]
    if !directory.join("skillverse.sqlite3").exists() {
        // Windows can redirect the roaming directory when the app starts under
        // the Codex Store package identity. A standalone launch can reopen it.
        let redirected = app
            .path()
            .local_data_dir()?
            .join("Packages/OpenAI.Codex_2p2nqsd0c76g0/LocalCache/Roaming")
            .join(app.config().identifier.as_str())
            .join("skillverse.sqlite3");
        if redirected.is_file() {
            return Ok(redirected);
        }
    }
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join("skillverse.sqlite3"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .on_window_event(|_, event| match event {
            tauri::WindowEvent::CloseRequested { .. } => lifecycle::note("WINDOW_CLOSE_REQUESTED"),
            tauri::WindowEvent::Destroyed => lifecycle::note("WINDOW_DESTROYED"),
            _ => {}
        })
        .setup(|app| {
            let path = app_database_path(app.handle()).ok();
            let new_profile = path
                .as_ref()
                .is_some_and(|path| matches!(path.try_exists(), Ok(false)));
            let database = path
                .and_then(|path| tauri::async_runtime::block_on(Database::open_desktop(path)).ok());
            if let Some(db) = &database {
                if tauri::async_runtime::block_on(db.recover_orchestration()).is_err() {
                    return Err("Workflow recovery failed; no workflow was resumed.".into());
                }
            }
            let event_handle = app.handle().clone();
            orchestration::set_event_sink(std::sync::Arc::new(move |event| {
                use tauri::Emitter;
                let _ = event_handle.emit("workflow:event", event);
            }));
            let github = database
                .as_ref()
                .and_then(|db| {
                    tauri::async_runtime::block_on(github::GitHubDiscoveryService::new(db.clone()))
                        .ok()
                })
                .map(Arc::new);
            let installer = database
                .as_ref()
                .zip(github.as_ref())
                .and_then(|(db, github)| {
                    let profile = if let Some(root) = isolated_installation_root() {
                        installer::types::Profile::isolated(root)
                    } else {
                        installer::types::Profile::production(PathBuf::from(
                            "D:/PersonalProject_DevTools/SkillVerseInstallations",
                        ))
                    }
                    .ok()?;
                    tauri::async_runtime::block_on(installer::Installer::new(
                        db.clone(),
                        profile,
                        github.clone(),
                    ))
                    .ok()
                    .map(Arc::new)
                });
            if isolated_installation_root().is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title("SkillVerse AI — ISOLATED INSTALLATION TEST");
                }
            }
            app.manage(GitHubState(github));
            app.manage(InstallerState(installer));
            app.manage(DatabaseState {
                database,
                new_profile,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workflow_engine,
            learning_engine,
            capability_intelligence,
            intelligence_credential,
            list_capabilities,
            get_capability,
            list_domains,
            get_capability_relations,
            save_graph_layout,
            load_graph_layout,
            get_installation_status,
            get_learning_progress,
            update_learning_progress,
            check_database_integrity,
            desktop_startup,
            create_database_backup,
            scan_codex,
            capability_reconciliation_preview,
            capability_reconciliation_apply,
            get_codex_scan,
            github_discovery_state,
            github_run,
            github_cancel,
            github_resume,
            github_set_token,
            github_classify,
            github_installation_plan,
            installation_state,
            installation_prepare,
            installation_operation,
            installation_execute,
            installation_cancel,
            installation_recover,
            installation_verify
        ])
        .build(tauri::generate_context!())
        .expect("failed to build SkillVerse AI")
        .run(|_, event| match event {
            tauri::RunEvent::Ready => lifecycle::note("READY"),
            tauri::RunEvent::ExitRequested { .. } => lifecycle::note("EXIT_REQUESTED"),
            tauri::RunEvent::Exit => lifecycle::note("EXIT"),
            _ => {}
        });
}
