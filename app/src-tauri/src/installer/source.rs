use super::{files, types::*};
use crate::{
    db::Database,
    github::{GitHubDiscoveryService, http::RequestContext},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
pub type Inventory = BTreeMap<String, (String, String)>;
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub trait PackageSource: Send + Sync {
    fn inventory<'a>(
        &'a self,
        repository: &'a str,
        commit: &'a str,
        root: &'a str,
        context: &'a RequestContext,
    ) -> Pending<'a, Inventory>;
    fn bytes<'a>(
        &'a self,
        source: &'a Source,
        file: &'a SourceFile,
        context: &'a RequestContext,
    ) -> Pending<'a, Vec<u8>>;
}
pub struct GitHubSource(pub Arc<GitHubDiscoveryService>);
impl PackageSource for GitHubSource {
    fn inventory<'a>(
        &'a self,
        repository: &'a str,
        commit: &'a str,
        root: &'a str,
        context: &'a RequestContext,
    ) -> Pending<'a, Inventory> {
        Box::pin(async move {
            self.0
                .installation_tree(repository, commit, root, context)
                .await
        })
    }
    fn bytes<'a>(
        &'a self,
        source: &'a Source,
        file: &'a SourceFile,
        context: &'a RequestContext,
    ) -> Pending<'a, Vec<u8>> {
        Box::pin(async move {
            self.0
                .installation_bytes(
                    &source.repository,
                    &file.path,
                    &source.commit,
                    &file.blob,
                    context,
                )
                .await
        })
    }
}
pub fn review_hash(data: &Value) -> String {
    hash(
        json!([
            data["compatibility"],
            data["reviewState"],
            data["risks"],
            data["latestCommit"],
            data["packageFingerprint"],
            data["packageRoot"]
        ])
        .to_string()
        .as_bytes(),
    )
}
pub async fn reviewed(db: &Database, id: &str) -> Result<(Value, crate::db::CapabilityRecord)> {
    let entity = db
        .get_capability(id)
        .await
        .map_err(Error::from)?
        .ok_or_else(|| fail("NOT_FOUND", "The capability is unavailable."))?;
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT payload_json FROM github_capability_intelligence WHERE entity_id=?",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await
    .map_err(|_| Error::database())?;
    let data: Value = raw
        .and_then(|v| serde_json::from_str(&v).ok())
        .ok_or_else(|| {
            fail(
                "REVIEW_REQUIRED",
                "Inspect the actual source in GitHub Live Intelligence before installation.",
            )
        })?;
    if ![
        "VERIFIED_COMPATIBLE",
        "LIKELY_COMPATIBLE",
        "REQUIRES_CONFIGURATION",
    ]
    .contains(&data["compatibility"].as_str().unwrap_or(""))
        || data["reviewState"] == "NEEDS_REVIEW"
        || data["sourceState"] != "PRESENT"
        || data["risks"]
            .as_array()
            .is_none_or(|risks| !risks.is_empty())
    {
        return Err(fail(
            "REVIEW_REQUIRED",
            "Source compatibility, availability, or static safety review is unresolved. Installation is blocked.",
        ));
    }
    if !["skill", "plugin", "mcp"].contains(&entity.kind.as_str()) {
        return Err(fail(
            "UNSUPPORTED",
            "A tool repository needs a reviewed Skill, Plugin, or MCP adapter.",
        ));
    }
    Ok((data, entity))
}
pub async fn snapshot(
    db: &Database,
    id: &str,
    provider: &dyn PackageSource,
    context: &RequestContext,
) -> Result<Source> {
    let (data, entity) = reviewed(db, id).await?;
    let commit = data["latestCommit"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| fail("REVISION", "A reviewed immutable commit is required."))?
        .to_string();
    let root = data["packageRoot"]
        .as_str()
        .ok_or_else(|| fail("INVENTORY", "The package root is unverified."))?
        .to_string();
    if !root.is_empty() {
        files::relative(&root)?;
    }
    let name = data["manifestName"]
        .as_str()
        .ok_or_else(|| {
            fail(
                "REVIEW_REQUIRED",
                "Refresh source inspection to capture the actual manifest identifier.",
            )
        })?
        .to_string();
    if entity.kind != "mcp" {
        files::identifier(&name)?;
    }
    let path: Option<String> =
        sqlx::query_scalar("SELECT source_path FROM github_capability_sources WHERE entity_id=?")
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .map_err(|_| Error::database())?;
    let manifest_path = path.ok_or_else(|| {
        fail(
            "IDENTITY",
            "A canonical repository/component association is required.",
        )
    })?;
    let repository = format!(
        "{}/{}",
        entity.source_owner.as_deref().unwrap_or(""),
        entity.source_repo.as_deref().unwrap_or("")
    );
    let tree = provider
        .inventory(&repository, &commit, &root, context)
        .await?;
    let fingerprint =
        crate::github::analysis::package_fingerprint(&root, &tree).ok_or_else(|| {
            fail(
                "INVENTORY",
                "The complete bounded package inventory is unavailable or contains links.",
            )
        })?;
    if data["packageFingerprint"] != fingerprint {
        return Err(fail(
            "SOURCE_CHANGED",
            "The source inventory differs from the reviewed package. Inspect it again and generate a new preview.",
        ));
    }
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{root}/")
    };
    let mut files_out = Vec::new();
    let mut names = std::collections::HashSet::new();
    for (path, (blob, mode)) in tree {
        let Some(relative) = path.strip_prefix(&prefix) else {
            continue;
        };
        let relative = files::relative(relative)?;
        if !names.insert(relative.to_lowercase()) {
            return Err(fail(
                "PATH_COLLISION",
                "Two source paths collide on a case-insensitive Windows filesystem.",
            ));
        }
        if !["100644", "100755"].contains(&mode.as_str()) {
            return Err(fail(
                "UNSAFE_FILE",
                "Links, submodules and unsupported file modes are blocked.",
            ));
        }
        let lower = relative.to_ascii_lowercase();
        if [
            ".exe", ".dll", ".msi", ".bat", ".cmd", ".com", ".scr", ".zip", ".7z", ".tar", ".gz",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext))
        {
            return Err(fail(
                "REVIEW_REQUIRED",
                "Executable binaries and nested archives need a dedicated reviewed adapter.",
            ));
        }
        if blob.len() != 40 || !blob.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(fail(
                "INTEGRITY",
                "The source contains an invalid blob identity.",
            ));
        }
        files_out.push(SourceFile {
            path,
            relative,
            blob,
            mode,
        });
    }
    if files_out.is_empty()
        || files_out.len() > files::MAX_FILES
        || !files_out.iter().any(|file| file.path == manifest_path)
    {
        return Err(fail(
            "INVENTORY",
            "The reviewed manifest is missing from the package.",
        ));
    }
    if files_out
        .iter()
        .any(|file| file.relative.to_lowercase().contains("hooks"))
    {
        return Err(fail(
            "HOOK_REVIEW",
            "Packages containing automatic lifecycle hooks require a separate security review; nothing will be enabled.",
        ));
    }
    let requirements = data["requirements"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(Source {
        repository,
        commit,
        manifest_path,
        package_root: root,
        name,
        kind: entity.kind,
        fingerprint,
        review_hash: review_hash(&data),
        files: files_out,
        requirements,
    })
}
