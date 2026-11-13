//! Local, bounded data inspection. No commands, uploaded code or tools are run.
use super::*;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub(super) fn ordinary(path: &Path) -> bool {
    path.ancestors().all(|p| {
        std::fs::symlink_metadata(p).is_ok_and(|m| {
            if m.file_type().is_symlink() {
                return false;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
            true
        })
    })
}
pub(super) fn root() -> PathBuf {
    #[cfg(test)]
    if let Ok(path) = std::env::var("SKILLVERSE_LEARNING_TEST_ROOT") {
        return PathBuf::from(path);
    }
    if let Some(path) = crate::isolated_installation_root() {
        return path.join("learning");
    }
    PathBuf::from("D:/PersonalProject_DevTools/SkillVerseLearning")
}
pub(super) fn read_artifact(workspace: &str, name: &str) -> Result<Vec<u8>> {
    let base = root();
    let workspace = PathBuf::from(workspace);
    if !workspace.starts_with(base.join("practice"))
        || workspace
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        || !ordinary(&workspace)
        || name.contains(['/', '\\', ':'])
        || name.starts_with('.')
    {
        return Err("ILLEGAL_EVIDENCE_PATH".into());
    }
    let path = workspace.join(name);
    if !ordinary(&path) {
        return Err("EVIDENCE_MISSING_OR_LINKED".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(&path).map_err(|_| "EVIDENCE_UNREADABLE")?;
    let meta = file.metadata().map_err(|_| "EVIDENCE_UNREADABLE")?;
    if !meta.is_file() || meta.len() > 262144 {
        return Err("EVIDENCE_SIZE_OR_TYPE".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut info = std::mem::MaybeUninit::<
            windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
        >::uninit();
        // The open file owns a valid borrowed handle; the OS initializes info on success.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
                file.as_raw_handle() as _,
                info.as_mut_ptr(),
            )
        };
        if ok == 0 || unsafe { info.assume_init() }.nNumberOfLinks != 1 {
            return Err("LINKED_EVIDENCE_REJECTED".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(262145)
        .read_to_end(&mut bytes)
        .map_err(|_| "EVIDENCE_UNREADABLE")?;
    if bytes.len() > 262144 {
        return Err("EVIDENCE_TOO_LARGE".into());
    }
    Ok(bytes)
}
pub(super) fn save_artifact(
    workspace: &str,
    name: &str,
    body: &str,
    expected: Option<&str>,
) -> Result<String> {
    if body.len() > 262144 {
        return Err("EVIDENCE_TOO_LARGE".into());
    }
    let dir = PathBuf::from(workspace);
    if !dir.starts_with(root().join("practice"))
        || !ordinary(&dir)
        || name.contains(['/', '\\', ':'])
        || name.starts_with('.')
    {
        return Err("ILLEGAL_EVIDENCE_PATH".into());
    }
    let path = dir.join(name);
    let exists = path.exists();
    if exists {
        let bytes = read_artifact(workspace, name)?;
        if expected != Some(digest(&bytes).as_str()) {
            return Err("ARTIFACT_CHANGED_RELOAD_BEFORE_SAVING".into());
        }
    } else if expected.is_some() {
        return Err("ARTIFACT_CHANGED_RELOAD_BEFORE_SAVING".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create_new(!exists);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x00200000);
    }
    let mut file = options.open(&path).map_err(|_| "ARTIFACT_SAVE_FAILED")?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut info = std::mem::MaybeUninit::<
            windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
        >::uninit();
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
                file.as_raw_handle() as _,
                info.as_mut_ptr(),
            )
        };
        if ok == 0 {
            return Err("ARTIFACT_METADATA_UNAVAILABLE".into());
        }
        let info = unsafe { info.assume_init() };
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0 {
            return Err("LINKED_ARTIFACT_REJECTED".into());
        }
    }
    use std::io::{Seek, Write};
    if exists {
        let mut current = Vec::new();
        (&mut file)
            .take(262145)
            .read_to_end(&mut current)
            .map_err(|_| "ARTIFACT_UNREADABLE")?;
        if expected != Some(digest(&current).as_str()) {
            return Err("ARTIFACT_CHANGED_RELOAD_BEFORE_SAVING".into());
        }
    }
    file.rewind().map_err(|_| "ARTIFACT_SAVE_FAILED")?;
    file.set_len(0)
        .and_then(|_| file.write_all(body.as_bytes()))
        .and_then(|_| file.sync_all())
        .map_err(|_| "ARTIFACT_SAVE_FAILED")?;
    Ok(digest(body.as_bytes()))
}
pub(super) fn workspace(session: &str, task: &Value) -> Result<PathBuf> {
    let base = root();
    std::fs::create_dir_all(&base).map_err(|_| "WORKSPACE_UNAVAILABLE")?;
    if !ordinary(&base) {
        return Err("WORKSPACE_LINK_REJECTED".into());
    }
    let path = base.join("practice").join(session);
    std::fs::create_dir_all(&path).map_err(|_| "WORKSPACE_UNAVAILABLE")?;
    if !ordinary(&path) {
        return Err("WORKSPACE_LINK_REJECTED".into());
    }
    use std::io::Write;
    for (name, content) in [
        ("input.txt", task["fixture"].as_str().unwrap_or("")),
        ("TASK.md", task["instructions"].as_str().unwrap_or("")),
    ] {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(name))
            .and_then(|mut f| f.write_all(content.as_bytes()))
            .map_err(|_| "WORKSPACE_CONFLICT")?;
    }
    Ok(path)
}
pub(super) fn seal(run: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = root().join("evidence");
    std::fs::create_dir_all(&dir).map_err(|_| "EVIDENCE_STORAGE")?;
    if !ordinary(&dir) {
        return Err("EVIDENCE_STORAGE_LINK".into());
    }
    let path = dir.join(format!("{run}.data"));
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "EVIDENCE_STORAGE")?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| "EVIDENCE_STORAGE")?;
    Ok(path)
}
pub(super) fn intact(path: &str, hash: &str) -> bool {
    let p = PathBuf::from(path);
    if !p.starts_with(root().join("evidence")) || !ordinary(&p) {
        return false;
    }
    let Ok(file) = std::fs::File::open(&p) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(262145).read_to_end(&mut bytes).is_err() {
        return false;
    }
    bytes.len() <= 262144 && digest(&bytes) == hash
}
pub(super) fn check(task: &Value, bytes: &[u8]) -> std::result::Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Artifact must be UTF-8 data.")?;
    let nonce = task["nonce"].as_str().ok_or("Challenge unavailable")?;
    if !text.contains(nonce) {
        return Err("Artifact does not belong to this practice challenge.".into());
    }
    if text.to_lowercase().contains("runtime_verified") || text.to_lowercase().contains("exit_code")
    {
        return Err("External execution claims cannot establish runtime evidence.".into());
    }
    let checks = match task["recipe"].as_str().unwrap_or("review") {
        "frontend" => {
            let low = text.to_lowercase();
            if !["<main", "<nav", "<section", "@media", "name=\"viewport\""]
                .iter()
                .all(|s| low.contains(s))
            {
                return Err(
                    "Provide main/nav/section, a viewport declaration and a responsive CSS rule."
                        .into(),
                );
            }
            vec![
                "HTML landmark declarations (source pattern checks only)",
                "Responsive source declarations (not a browser behavior test)",
            ]
        }
        "analytics" => {
            let data: Value = serde_json::from_str(text).map_err(|_| "Expected JSON result")?;
            let values: Vec<i64> = task["fixture"]
                .as_str()
                .unwrap_or("")
                .lines()
                .filter_map(|line| line.parse::<i64>().ok())
                .collect();
            if values.len() != 3 {
                return Err("The trusted fixture is invalid.".into());
            }
            if data["nonce"] != task["nonce"]
                || data["total"] != json!(values.iter().sum::<i64>())
                || data["row_count"] != 3
            {
                return Err(
                    "The independently computed fixture total or row count does not match.".into(),
                );
            }
            vec![
                "Fixture total independently recomputed",
                "Three input rows accounted for",
            ]
        }
        "browser" => {
            let data: Value =
                serde_json::from_str(text).map_err(|_| "Expected a JSON test plan")?;
            if data["nonce"] != task["nonce"] {
                return Err("Challenge nonce does not match.".into());
            }
            let cases = data["cases"].as_array().ok_or("Missing cases")?;
            if cases.len() < 3
                || cases.iter().any(|c| {
                    !["selector", "action", "expected"]
                        .iter()
                        .all(|k| c[*k].as_str().is_some_and(|s| !s.trim().is_empty()))
                })
            {
                return Err("Describe three selector/action/expected cases; this does not prove test execution.".into());
            }
            vec![
                "Three explicit test cases",
                "Selectors, actions and assertions described (not executed)",
            ]
        }
        "presentations" => {
            if text.lines().filter(|l| l.starts_with("# ")).count() < 3 {
                return Err("Provide at least three slide headings.".into());
            }
            vec!["Three-slide outline", "Session-bound source attribution"]
        }
        "video" => {
            let data: Value = serde_json::from_str(text).map_err(|_| "Expected storyboard JSON")?;
            if data["nonce"] != task["nonce"] {
                return Err("Challenge nonce does not match.".into());
            }
            let scenes = data["scenes"].as_array().ok_or("Missing scenes")?;
            if scenes.len() < 3
                || scenes.iter().any(|s| {
                    s["endSeconds"].as_f64().unwrap_or(0.)
                        <= s["startSeconds"].as_f64().unwrap_or(0.)
                        || s["description"].as_str().unwrap_or("").is_empty()
                })
            {
                return Err("Provide three scenes with valid time ranges and descriptions.".into());
            }
            vec![
                "Storyboard time ranges",
                "Scene descriptions (no video rendered)",
            ]
        }
        _ => {
            if !["# Summary", "# Evidence", "# Limitations"]
                .iter()
                .all(|s| text.contains(s))
                || !text.contains("Learning fixtures contain no private files.")
            {
                return Err("Include Summary, Evidence and Limitations headings and cite the exact fixture statement.".into());
            }
            vec![
                "Required report structure",
                "Exact fixture statement extracted",
            ]
        }
    };
    Ok(checks.into_iter().map(String::from).collect())
}
