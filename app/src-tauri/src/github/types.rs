use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub kind: String,
    pub message: String,
    pub retry_at: Option<i64>,
    pub status: Option<u16>,
}
impl ApiError {
    pub fn new(kind: &str, message: &str) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            retry_at: None,
            status: None,
        }
    }
    pub fn database() -> Self {
        Self::new(
            "DATABASE",
            "GitHub data could not be saved. Existing records were retained.",
        )
    }
}
impl From<String> for ApiError {
    fn from(_: String) -> Self {
        Self::database()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: i64,
    pub full_name: String,
    pub description: String,
    pub stars: i64,
    pub forks: i64,
    pub default_branch: String,
    pub archived: bool,
    pub disabled: bool,
    pub license: Option<String>,
    pub pushed_at: Option<String>,
    pub updated_at: Option<String>,
    pub topics: Vec<String>,
    pub html_url: String,
}
impl Repository {
    pub fn parse(value: &Value) -> Result<Self, ApiError> {
        if value["private"].as_bool() == Some(true) {
            return Err(ApiError::new(
                "PRIVATE",
                "This phase only inspects public repositories.",
            ));
        }
        let id = value["id"].as_i64().filter(|id| *id > 0).ok_or_else(|| {
            ApiError::new(
                "INVALID_RESPONSE",
                "GitHub returned an invalid repository identity.",
            )
        })?;
        let full_name = normalize_repository(value["full_name"].as_str().unwrap_or(""))?;
        Ok(Self {
            id,
            html_url: format!("https://github.com/{full_name}"),
            full_name,
            description: value["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(2000)
                .collect(),
            stars: value["stargazers_count"].as_i64().unwrap_or(0),
            forks: value["forks_count"].as_i64().unwrap_or(0),
            default_branch: value["default_branch"].as_str().unwrap_or("main").into(),
            archived: value["archived"].as_bool().unwrap_or(false),
            disabled: value["disabled"].as_bool().unwrap_or(false),
            license: value["license"]["spdx_id"]
                .as_str()
                .filter(|s| *s != "NOASSERTION")
                .map(str::to_string),
            pushed_at: value["pushed_at"].as_str().map(str::to_string),
            updated_at: value["updated_at"].as_str().map(str::to_string),
            topics: value["topics"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .take(40)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}
pub fn normalize_repository(input: &str) -> Result<String, ApiError> {
    let raw = input.trim();
    let name = if raw.starts_with("https://github.com/") {
        raw.trim_start_matches("https://github.com/")
            .trim_end_matches('/')
    } else {
        raw
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || s.len() > 100
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
    {
        return Err(ApiError::new(
            "INVALID_REPOSITORY",
            "Enter owner/repository or its canonical https://github.com URL.",
        ));
    }
    Ok(name.to_ascii_lowercase())
}
pub fn safe_path(path: &str) -> Result<String, ApiError> {
    let clean = path
        .strip_prefix("./")
        .unwrap_or(path)
        .trim_end_matches('/');
    if clean.len() > 700
        || clean.is_empty()
        || clean.starts_with('/')
        || clean.contains(['\\', ':', '\0', '%'])
        || clean.chars().any(char::is_control)
        || clean
            .split('/')
            .any(|part| part.ends_with(['.', ' ']) || part.contains(['*', '?', '"', '<', '>', '|']))
        || clean
            .split('/')
            .any(|p| p == ".." || p == "." || p.is_empty())
    {
        return Err(ApiError::new(
            "UNSAFE_PATH",
            "A repository path escapes or is not a supported relative path.",
        ));
    }
    Ok(clean.trim_end_matches('/').into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub primary: Option<String>,
    pub related: Vec<String>,
    pub state: String,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub manual: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub kind: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub blob_sha: String,
    pub content_hash: String,
    pub compatibility: String,
    pub evidence: Vec<String>,
    pub requirements: Vec<String>,
    pub risks: Vec<String>,
    pub review_state: String,
    pub classification: Classification,
    pub package_fingerprint: Option<String>,
    pub package_root: String,
    pub package_files: Vec<String>,
    pub manifest_version: Option<String>,
    pub dependencies: Vec<String>,
    pub resource_paths: Vec<String>,
    pub quality: Value,
}
impl Assessment {
    pub fn intelligence(&self, commit: &str, checked: i64) -> Value {
        json!({"manifestName":self.name,"compatibility":self.compatibility,"compatibilityEvidence":self.evidence,"requirements":self.requirements,"risks":self.risks,"reviewState":self.review_state,"quality":self.quality,"subcategory":super::analysis::subcategory(&self.name,&self.description,self.classification.primary.as_deref()),"checkedAt":checked,"latestCommit":commit,"latestBlob":self.blob_sha,"dependencies":self.dependencies,"resourcePaths":self.resource_paths,"packageRoot":self.package_root,"packageFiles":self.package_files,"contentHash":self.content_hash,"packageFingerprint":self.package_fingerprint,"remoteVersion":self.manifest_version})
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Operation {
    pub action: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub page: Option<u16>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub body: Value,
    pub source: String,
    pub fetched_at: i64,
    pub validated_at: i64,
    pub warning: Option<ApiError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_identity_and_paths_are_restricted() {
        assert_eq!(
            normalize_repository("https://github.com/OpenAI/Skills.git").unwrap(),
            "openai/skills"
        );
        for input in [
            "http://github.com/a/b",
            "https://evil.test/a/b",
            "a/b/../c",
            "a/b?token=x",
        ] {
            assert!(normalize_repository(input).is_err());
        }
        for path in [
            "../config.toml",
            "/etc/passwd",
            "C:\\secret",
            "a/%2e%2e/b",
            "a//b",
        ] {
            assert!(safe_path(path).is_err());
        }
    }
}
