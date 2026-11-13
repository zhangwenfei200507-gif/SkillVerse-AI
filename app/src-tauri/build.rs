fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bin=skillverse-ai=/Brepro");
    }

    const COMMANDS: &[&str] = &[
        "workflow_engine",
        "learning_engine",
        "capability_intelligence",
        "intelligence_credential",
        "list_capabilities",
        "get_capability",
        "list_domains",
        "get_capability_relations",
        "save_graph_layout",
        "load_graph_layout",
        "get_installation_status",
        "get_learning_progress",
        "update_learning_progress",
        "check_database_integrity",
        "desktop_startup",
        "create_database_backup",
        "scan_codex",
        "capability_reconciliation_preview",
        "capability_reconciliation_apply",
        "get_codex_scan",
        "github_discovery_state",
        "github_run",
        "github_cancel",
        "github_resume",
        "github_set_token",
        "github_classify",
        "github_installation_plan",
        "installation_state",
        "installation_prepare",
        "installation_operation",
        "installation_execute",
        "installation_cancel",
        "installation_recover",
        "installation_verify",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("build the SkillVerse command capability");
}
