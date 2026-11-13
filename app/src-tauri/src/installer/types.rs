use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
pub type Error = crate::github::types::ApiError;
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub fn fail(kind: &str, message: &str) -> Error {
    Error::new(kind, message)
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn hash(bytes: &[u8]) -> String {
    crate::github::types::digest(bytes)
}
pub fn time() -> i64 {
    crate::github::types::now()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub isolated: bool,
    pub user_home: PathBuf,
    pub codex_home: PathBuf,
    pub project_root: PathBuf,
    pub work_root: PathBuf,
}
impl Profile {
    pub fn isolated(root: PathBuf) -> Result<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|part| part == std::path::Component::ParentDir)
            || (cfg!(windows)
                && !root
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("d:"))
        {
            return Err(fail(
                "ISOLATION",
                "Isolated development data must use an absolute D: path.",
            ));
        }
        let existing_ancestor = root.ancestors().find(|path| path.exists());
        if !existing_ancestor.is_some_and(crate::orchestration::is_ordinary_path) {
            return Err(fail(
                "ISOLATION",
                "The test root must not traverse a link or reparse point.",
            ));
        }
        if let Some(real) = crate::codex_scanner::resolve_codex_home() {
            let normalized = |p: &std::path::Path| {
                p.to_string_lossy()
                    .replace('\\', "/")
                    .trim_end_matches('/')
                    .to_lowercase()
            };
            let (candidate, home) = (normalized(&root), normalized(&real));
            if candidate == home
                || candidate.starts_with(&format!("{home}/"))
                || home.starts_with(&format!("{candidate}/"))
            {
                return Err(fail(
                    "ISOLATION",
                    "The test root overlaps the real Codex home.",
                ));
            }
        }
        Ok(Self {
            id: hash(root.to_string_lossy().to_lowercase().as_bytes())[..24].into(),
            isolated: true,
            user_home: root.join("user"),
            codex_home: root.join("codex"),
            project_root: root.join("project"),
            work_root: root.join("transactions"),
        })
    }
    pub fn production(work_root: PathBuf) -> Result<Self> {
        let user_home = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .ok_or_else(|| fail("PROFILE", "The user home is unavailable."))?;
        let codex_home = crate::codex_scanner::resolve_codex_home()
            .ok_or_else(|| fail("PROFILE", "Codex home is unavailable."))?;
        let cwd =
            std::env::current_dir().map_err(|_| fail("PROFILE", "The project is unavailable."))?;
        let project_root = cwd
            .ancestors()
            .find(|p| p.join(".git").exists())
            .unwrap_or(&cwd)
            .to_path_buf();
        Ok(Self {
            id: hash(
                format!("{}|{}", codex_home.display(), project_root.display())
                    .to_lowercase()
                    .as_bytes(),
            )[..24]
                .into(),
            isolated: false,
            user_home,
            codex_home,
            project_root,
            work_root,
        })
    }
    pub fn approved_root(&self, path: &std::path::Path) -> Option<PathBuf> {
        [
            &self.user_home,
            &self.codex_home,
            &self.project_root,
            &self.work_root,
        ]
        .into_iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub entity_id: String,
    pub action: String,
    #[serde(default = "user_scope")]
    pub scope: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub rollback_operation: Option<String>,
    #[serde(default)]
    pub mcp: Option<McpDraft>,
}
fn user_scope() -> String {
    "user".into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDraft {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub environment_names: Vec<String>,
    #[serde(default)]
    pub bearer_token_environment: Option<String>,
    #[serde(default)]
    pub auth_required: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub path: String,
    pub relative: String,
    pub blob: String,
    pub mode: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub repository: String,
    pub commit: String,
    pub manifest_path: String,
    pub package_root: String,
    pub name: String,
    pub kind: String,
    pub fingerprint: String,
    pub review_hash: String,
    pub files: Vec<SourceFile>,
    pub requirements: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub kind: String,
    pub fingerprint: String,
    pub files: Vec<FileInfo>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub format: String,
    pub section: String,
    pub key: String,
    pub expected_hash: String,
    pub value: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub target: PathBuf,
    pub staging_path: PathBuf,
    pub holding_path: PathBuf,
    pub role: String,
    pub before: Image,
    pub patch: Option<Patch>,
    pub restore_from: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub operation_id: String,
    pub profile: Profile,
    pub entity_id: String,
    pub action: String,
    pub scope: String,
    pub kind: String,
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_changes: Vec<Value>,
    pub managed_id: Option<String>,
    pub changes: Vec<Change>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub requirements: Vec<String>,
    pub prior_record: Option<Record>,
    pub created_at: i64,
    pub expires_at: i64,
    pub rollback_operation: Option<String>,
    pub dry_run: bool,
    pub can_execute: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub entity_id: String,
    pub profile_id: String,
    pub kind: String,
    pub scope: String,
    pub target: PathBuf,
    pub source: Option<Source>,
    pub state: String,
    pub verification: Vec<String>,
    pub local_entity_id: Option<String>,
    pub current_operation_id: String,
    pub targets: Vec<OwnedTarget>,
    pub patches: Vec<OwnedPatch>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedTarget {
    pub target: PathBuf,
    pub image: Image,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedPatch {
    pub target: PathBuf,
    pub patch: Patch,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: String,
    pub state: String,
    pub plan: Plan,
    pub digest: String,
    pub error: Option<Error>,
    pub updated_at: i64,
    pub recovery: Value,
}
