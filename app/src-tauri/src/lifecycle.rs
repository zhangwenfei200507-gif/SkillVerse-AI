//! Lifecycle diagnostics contain event names only, never user data.
pub(crate) fn note(event: &str) {
    let Ok(path) = std::env::var("SKILLVERSE_LIFECYCLE_LOG") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() || !path.to_string_lossy().to_lowercase().starts_with("d:") {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            file,
            "{}",
            serde_json::json!({"event":event,"pid":std::process::id(),"time":crate::github::types::now()})
        );
    }
}
