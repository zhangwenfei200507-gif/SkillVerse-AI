use serde::{Deserialize, Serialize};
use sqlx::{
    FromRow, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::path::PathBuf;

const DATABASE_ERROR: &str =
    "Local database operation failed. Your existing data was not intentionally removed.";
const MIGRATION_0001: &str = include_str!("../migrations/0001_graph_data.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_codex_scan_state.sql");
const MIGRATION_0003: &str = include_str!("../migrations/0003_github_discovery_cache.sql");
const MIGRATION_0004: &str = include_str!("../migrations/0004_github_component_paths.sql");
const MIGRATION_0005: &str = include_str!("../migrations/0005_github_live_intelligence.sql");
const MIGRATION_0006: &str = include_str!("../migrations/0006_installation_transactions.sql");
const DISCOVERY_CATALOG: &str = include_str!("../../src/domain/discovery-catalog.json");

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: SqlitePool,
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    pub source_type: Option<String>,
    pub source_owner: Option<String>,
    pub source_repo: Option<String>,
    pub source_ref: Option<String>,
    pub source_url: Option<String>,
    pub license_spdx: Option<String>,
    pub metadata_json: String,
    pub primary_category_id: Option<String>,
    pub primary_category_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DomainRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub slug: String,
    pub name: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RelationRecord {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct GraphPosition {
    pub id: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityInput {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    pub source_type: Option<String>,
    pub source_owner: Option<String>,
    pub source_repo: Option<String>,
    pub source_ref: Option<String>,
    pub source_url: Option<String>,
    pub license_spdx: Option<String>,
    pub metadata: serde_json::Value,
    pub primary_category_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphLayoutInput {
    pub profile: String,
    pub positions: Vec<GraphPosition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstallationStatus {
    Unknown,
    NotInstalled,
    Installed,
    Conflict,
    Unsupported,
    Error,
}

impl InstallationStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::NotInstalled => "NOT_INSTALLED",
            Self::Installed => "INSTALLED",
            Self::Conflict => "CONFLICT",
            Self::Unsupported => "UNSUPPORTED",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InstallationRecord {
    pub id: String,
    pub entity_id: String,
    pub status: String,
    pub source: Option<String>,
    pub version: Option<String>,
    pub observed_at: Option<String>,
    pub evidence_json: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallationInput {
    pub id: String,
    pub entity_id: String,
    pub status: InstallationStatus,
    pub source: Option<String>,
    pub version: Option<String>,
    pub observed_at: Option<String>,
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LearningRecord {
    pub id: String,
    pub entity_id: String,
    pub status: String,
    pub evidence_json: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LearningStatus {
    NotLearned,
    Exploring,
    Practicing,
    Verified,
    SelfReported,
}

impl LearningStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::NotLearned => "NOT_LEARNED",
            Self::Exploring => "EXPLORING",
            Self::Practicing => "PRACTICING",
            Self::Verified => "VERIFIED",
            Self::SelfReported => "SELF_REPORTED",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LearningInput {
    pub id: String,
    pub entity_id: String,
    pub status: LearningStatus,
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditEventInput {
    pub id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub outcome: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

impl Database {
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub async fn open_read_only(path: PathBuf) -> Result<Self, String> {
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| "Cannot open a read-only database.".to_string())?;
        Ok(Self { pool, path })
    }

    async fn backup_before_migration(
        pool: &SqlitePool,
        path: &std::path::Path,
        next_version: i64,
    ) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("Database backup directory is unavailable.")?;
        let directory = parent.join("backups");
        std::fs::create_dir_all(&directory)
            .map_err(|_| "Migration backup directory could not be created.")?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let snapshot = directory.join(format!("before-schema-{next_version}-{nonce}.sqlite3"));
        sqlx::query("VACUUM INTO ?")
            .bind(snapshot.to_string_lossy().as_ref())
            .execute(pool)
            .await
            .map_err(|_| "Migration backup failed; migration was not started.")?;
        let verify = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(snapshot)
                    .read_only(true),
            )
            .await
            .map_err(|_| "Migration backup could not be reopened.")?;
        let result: Result<String, _> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&verify)
            .await;
        verify.close().await;
        if result.ok().as_deref() != Some("ok") {
            return Err("Migration backup failed integrity verification.".into());
        }
        Ok(())
    }

    pub async fn open(path: PathBuf) -> Result<Self, String> {
        Self::open_with_bootstrap(path, false).await
    }

    pub(crate) async fn open_desktop(path: PathBuf) -> Result<Self, String> {
        Self::open_with_bootstrap(path, true).await
    }

    async fn open_with_bootstrap(path: PathBuf, latest_for_empty: bool) -> Result<Self, String> {
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Full);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;

        let setup = async {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        if version == 0 && latest_for_empty {
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(|_| DATABASE_ERROR.to_string())?;
            let version: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(&mut *tx).await
                .map_err(|_| DATABASE_ERROR.to_string())?;
            if version == 0 {
                let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')")
                    .fetch_one(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
                if occupied {
                    return Err("Database has unversioned objects; initialization stopped to protect existing data.".into());
                }
                let scripts = [
                    MIGRATION_0001, MIGRATION_0002, MIGRATION_0003, MIGRATION_0004, MIGRATION_0005, MIGRATION_0006,
                    include_str!("../migrations/0007_logical_capability_identities.sql"),
                    include_str!("../migrations/0008_capability_intelligence.sql"),
                    include_str!("../migrations/0009_learning_verification.sql"),
                    include_str!("../migrations/0010_orchestration.sql"),
                    include_str!("../migrations/0011_project_routing.sql"),
                    include_str!("../migrations/0012_model_registry.sql"),
                    include_str!("../migrations/0013_live_budget.sql"),
                ];
                for script in scripts {
                    // Legacy migration files own a transaction when run individually.
                    let body = script.lines().filter(|line| !matches!(line.trim(), "BEGIN;" | "BEGIN IMMEDIATE;" | "COMMIT;"))
                        .collect::<Vec<_>>().join("\n");
                    sqlx::raw_sql(&body).execute(&mut *tx).await
                        .map_err(|_| "Empty database initialization failed and was rolled back.".to_string())?;
                }
                if sqlx::query("PRAGMA foreign_key_check").fetch_optional(&mut *tx).await
                    .map_err(|_| DATABASE_ERROR.to_string())?.is_some() {
                    return Err("Empty database failed its foreign-key check.".into());
                }
            } else if version != 13 {
                return Err("Database was initialized by another application instance; reopen it before proceeding.".into());
            }
            tx.commit().await.map_err(|_| DATABASE_ERROR.to_string())?;
            Self::seed_domains(&pool).await?;
            return Ok(());
        }
        match version {
            0 => {
                let has_user_tables: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%')",
                )
                .fetch_one(&pool)
                .await
                .map_err(|_| DATABASE_ERROR.to_string())?;
                if has_user_tables {
                    return Err("Database has unversioned tables; migration stopped to protect existing data.".into());
                }
                sqlx::raw_sql(MIGRATION_0001)
                    .execute(&pool)
                    .await
                    .map_err(|_| "Could not apply local database migration 0001; existing files were left in place.".to_string())?;
                sqlx::raw_sql(MIGRATION_0002)
                    .execute(&pool)
                    .await
                    .map_err(|_| "Could not apply local database migration 0002; existing files were left in place.".to_string())?;
                sqlx::raw_sql(MIGRATION_0003).execute(&pool).await.map_err(|_| "Could not apply local database migration 0003; existing files were left in place.".to_string())?;
            }
            1 => {
                let migration_exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 1 AND name = 'graph_data')",
                )
                .fetch_one(&pool)
                .await
                .map_err(|_| "Database version marker is inconsistent; migration stopped to protect existing data.".to_string())?;
                if !migration_exists {
                    return Err("Database version marker is inconsistent; migration stopped to protect existing data.".into());
                }
                sqlx::raw_sql(MIGRATION_0002)
                    .execute(&pool)
                    .await
                    .map_err(|_| "Could not apply local database migration 0002; existing files were left in place.".to_string())?;
                sqlx::raw_sql(MIGRATION_0003).execute(&pool).await.map_err(|_| "Could not apply local database migration 0003; existing files were left in place.".to_string())?;
            }
            2 => {
                let migrations_exist: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 1 AND name = 'graph_data') AND EXISTS(SELECT 1 FROM schema_migrations WHERE version = 2 AND name = 'codex_scan_state')",
                )
                .fetch_one(&pool)
                .await
                .map_err(|_| "Database version marker is inconsistent; migration stopped to protect existing data.".to_string())?;
                if !migrations_exist {
                    return Err("Database version marker is inconsistent; migration stopped to protect existing data.".into());
                }
                sqlx::raw_sql(MIGRATION_0003).execute(&pool).await.map_err(|_| "Could not apply local database migration 0003; existing files were left in place.".to_string())?;
            }
            3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 => {
                let migrations_exist: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 1 AND name = 'graph_data') AND EXISTS(SELECT 1 FROM schema_migrations WHERE version = 2 AND name = 'codex_scan_state') AND EXISTS(SELECT 1 FROM schema_migrations WHERE version = 3 AND name = 'github_discovery_cache')",
                ).fetch_one(&pool).await.map_err(|_| "Database version marker is inconsistent; migration stopped to protect existing data.".to_string())?;
                if !migrations_exist {
                    return Err("Database version marker is inconsistent; migration stopped to protect existing data.".into());
                }
                if version >= 4 {
                    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=4 AND name='github_component_paths')")
                        .fetch_one(&pool).await.map_err(|_| DATABASE_ERROR.to_string())?;
                    if !valid {
                        return Err("Database version marker is inconsistent; migration stopped to protect existing data.".into());
                    }
                }
                if version >= 5 {
                    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=5 AND name='github_live_intelligence')").fetch_one(&pool).await.map_err(|_| DATABASE_ERROR.to_string())?;
                    if !valid { return Err("Database version marker is inconsistent; migration stopped.".into()); }
                }
            }
            _ => {
                return Err(format!(
                    "Database schema version {version} is newer than this application supports."
                ));
            }
        }

        if version < 4 {
            sqlx::raw_sql(MIGRATION_0004).execute(&pool).await.map_err(|_| "Could not apply local database migration 0004; existing files were left in place.".to_string())?;
        }
        if version < 5 {
            // A startup migration always has a verified, independently readable
            // snapshot, in addition to the operator's Phase 5 audit backup.
            Self::backup_before_migration(&pool, &path, 5).await?;
            if sqlx::raw_sql(MIGRATION_0005).execute(&pool).await.is_err() {
                let _ = sqlx::query("ROLLBACK").execute(&pool).await;
                return Err("Migration 0005 failed and was rolled back; the existing database and migration backup were retained.".into());
            }
        }
        if version < 6 {
            Self::backup_before_migration(&pool,&path,6).await?;
            if sqlx::raw_sql(MIGRATION_0006).execute(&pool).await.is_err() {let _=sqlx::query("ROLLBACK").execute(&pool).await;return Err("Migration 0006 failed and was rolled back; the verified backup was retained.".into());}
        }else if !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=6 AND name='installation_transactions')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? {return Err("Installation migration marker is inconsistent.".into());}
        if version>=7 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=7 AND name='logical_capability_identities')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? {return Err("Identity schema marker is inconsistent.".into());}
if version>=8 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=8 AND name='capability_intelligence')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? { return Err("Intelligence schema marker is inconsistent.".into()); }
if version>=9 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=9 AND name='learning_verification')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())?{return Err("Learning schema marker is inconsistent.".into());}
        if version>=10 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=10 AND name='autonomous_orchestration')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? { return Err("Orchestration schema marker is inconsistent.".into()); }
        if version>=11 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=11 AND name='project_routing')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? { return Err("Project routing schema marker is inconsistent.".into()); }
        if version>=12 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=12 AND name='model_registry')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? { return Err("Model registry schema marker is inconsistent.".into()); }
        if version>=13 && !sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=13 AND name='live_budget')").fetch_one(&pool).await.map_err(|_|DATABASE_ERROR.to_string())? { return Err("Live budget schema marker is inconsistent.".into()); }
        Self::seed_domains(&pool).await?;
        Ok::<(), String>(())
        }.await;
        if let Err(error) = setup {
            // SQLite workers must finish before reporting a failed open, so a
            // rejected database is immediately available for backup/inspection.
            pool.close().await;
            return Err(error);
        }
        Ok(Self { pool, path })
    }

    async fn seed_domains(pool: &SqlitePool) -> Result<(), String> {
        let domains = [
            ("Engineering", "engineering", "Engineering"),
            ("Knowledge", "knowledge", "Knowledge"),
            ("Automation", "automation", "Automation"),
            ("Frontend", "frontend", "Frontend"),
        ];
        let mut tx = pool.begin().await.map_err(|_| DATABASE_ERROR.to_string())?;
        sqlx::query("INSERT INTO graph_nodes(id, kind, layout_profile) VALUES ('codex-root', 'core', 'world-tree') ON CONFLICT DO NOTHING")
            .execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        for (id, slug, name) in domains {
            sqlx::query("INSERT INTO categories(id, slug, name, origin) VALUES (?, ?, ?, 'seed') ON CONFLICT(id) DO NOTHING")
                .bind(id).bind(slug).bind(name).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            sqlx::query("INSERT INTO graph_nodes(id, kind, category_id, layout_profile) VALUES (?, 'category', ?, 'world-tree') ON CONFLICT DO NOTHING")
                .bind(format!("branch-{name}")).bind(id).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        }
        let seed_entities = [
            (
                "github-mcp",
                "mcp",
                "GitHub MCP",
                "GitHub’s official MCP server for connecting an agent to GitHub context and workflows.",
                "github",
                "github-mcp-server",
                "Engineering",
            ),
            (
                "context7",
                "mcp",
                "Context7",
                "A documentation platform that provides current library references for AI agents.",
                "upstash",
                "context7",
                "Knowledge",
            ),
            (
                "playwright-mcp",
                "mcp",
                "Playwright MCP",
                "An MCP server that exposes browser automation through Playwright.",
                "microsoft",
                "playwright-mcp",
                "Automation",
            ),
            (
                "markitdown",
                "capability",
                "MarkItDown",
                "A Python tool for converting files and office documents to Markdown.",
                "microsoft",
                "markitdown",
                "Knowledge",
            ),
            (
                "superpowers",
                "skill",
                "Superpowers",
                "An agentic skills framework and software development methodology.",
                "obra",
                "superpowers",
                "Engineering",
            ),
            (
                "vercel-agent-skills",
                "skill",
                "Vercel Agent Skills",
                "Vercel’s collection of agent skills. Repository license has not been verified.",
                "vercel-labs",
                "agent-skills",
                "Frontend",
            ),
        ];
        for (id, kind, name, summary, owner, repo, category) in seed_entities {
            sqlx::query("INSERT INTO entities(id, kind, name, summary, source_type, source_owner, source_repo, source_url) VALUES (?, ?, ?, ?, 'github', ?, ?, ?) ON CONFLICT(id) DO NOTHING")
                .bind(id).bind(kind).bind(name).bind(summary).bind(owner).bind(repo).bind(format!("https://github.com/{owner}/{repo}"))
                .execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            sqlx::query("INSERT INTO entity_categories(entity_id, category_id, role, assigned_by) VALUES (?, ?, 'primary', 'manual') ON CONFLICT(entity_id, category_id) DO NOTHING")
                .bind(id).bind(category).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            sqlx::query("INSERT INTO graph_nodes(id, kind, entity_id, layout_profile) VALUES (?, 'entity', ?, 'world-tree') ON CONFLICT DO NOTHING")
                .bind(id).bind(id).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        }
        // This reviewed GitHub snapshot uses the existing entities/metadata schema.
        // It never writes scanner installation evidence or local Codex configuration.
        let catalog: serde_json::Value = serde_json::from_str(DISCOVERY_CATALOG)
            .map_err(|_| "The bundled discovery catalog is invalid.".to_string())?;
        let entries = catalog["items"]
            .as_array()
            .ok_or_else(|| "The bundled discovery catalog has no entries.".to_string())?;
        for item in entries {
            let field = |name: &str| -> Result<&str, String> {
                item[name].as_str().ok_or_else(|| {
                    "The bundled discovery catalog has invalid metadata.".to_string()
                })
            };
            let id = field("id")?;
            let metadata = serde_json::json!({ "discovery": item }).to_string();
            sqlx::query("INSERT INTO entities(id, kind, name, summary, source_type, source_owner, source_repo, source_ref, source_url, license_spdx, metadata_json, source_path) VALUES (?, ?, ?, ?, 'github', ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING")
                .bind(id).bind(field("kind")?).bind(field("name")?).bind(field("summary")?)
                .bind(field("owner")?).bind(field("repo")?).bind(field("sourceRef")?)
                .bind(field("githubUrl")?).bind(field("license")?).bind(metadata).bind(field("sourcePath")?)
                .execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            sqlx::query("INSERT INTO entity_categories(entity_id, category_id, role, assigned_by) VALUES (?, ?, 'primary', 'manual') ON CONFLICT(entity_id, category_id) DO NOTHING")
                .bind(id).bind(field("category")?).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        }
        tx.commit().await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn persist_codex_scan(
        &self,
        report: &crate::codex_scanner::CodexScanReport,
    ) -> Result<(), String> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        self.persist_codex_scan_in(&mut tx, report).await?;
        tx.commit().await.map_err(|_| DATABASE_ERROR.to_string())
    }
    pub(crate) async fn persist_codex_scan_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        report: &crate::codex_scanner::CodexScanReport,
    ) -> Result<(), String> {
        if report.entities.len() > 2000 {
            return Err("Codex scan produced more than 2,000 entries; nothing was saved.".into());
        }
        let current_ids: Vec<&str> = report
            .entities
            .iter()
            .map(|entity| entity.id.as_str())
            .collect();
        for entity in &report.entities {
            validate_text(&entity.id, "scanned entity id", 180)?;
        }
        let current_ids_json = serde_json::to_string(&current_ids)
            .map_err(|_| "Scanner result is invalid.".to_string())?;
        let identity_ready =
            report.identity.is_some() && crate::capability_identity::schema_ready(&mut *tx).await?;
        if !identity_ready {
            sqlx::query("UPDATE installations SET status='NOT_INSTALLED', observed_at=?, evidence_json=json_object('scannerObserved', false, 'reason', 'not found in latest scan'), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE source='scanner' AND entity_id NOT IN (SELECT value FROM json_each(?))")
            .bind(report.scanned_at_unix_ms.to_string()).bind(current_ids_json)
            .execute(&mut **tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        }
        for entity in &report.entities {
            validate_text(&entity.name, "scanned entity name", 240)?;
            if !matches!(entity.kind.as_str(), "skill" | "mcp" | "plugin") {
                return Err("Scanner returned an unsupported entity kind.".into());
            }
            let metadata = serde_json::to_string(&redact_sensitive(entity.evidence.clone()))
                .map_err(|_| "Scanner evidence is invalid.".to_string())?;
            sqlx::query("INSERT INTO entities(id, kind, name, summary, source_type, metadata_json) VALUES (?, ?, ?, ?, 'local', ?) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, summary=excluded.summary, source_type='local', metadata_json=json_patch(entities.metadata_json,excluded.metadata_json), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
                .bind(&entity.id).bind(&entity.kind).bind(&entity.name).bind(&entity.summary).bind(metadata)
                .execute(&mut **tx).await.map_err(|_| "Could not save scanned Codex metadata.".to_string())?;
            sqlx::query("INSERT INTO graph_nodes(id, kind, entity_id, layout_profile) VALUES (?, 'entity', ?, 'world-tree') ON CONFLICT(id) DO NOTHING")
                .bind(&entity.id).bind(&entity.id).execute(&mut **tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            let status = if entity.kind == "mcp" {
                InstallationStatus::Unknown
            } else {
                entity.status
            };
            let evidence = serde_json::to_string(&redact_sensitive(entity.evidence.clone()))
                .map_err(|_| "Scanner evidence is invalid.".to_string())?;
            sqlx::query("INSERT INTO installations(id, entity_id, status, source, observed_at, evidence_json) VALUES (?, ?, ?, 'scanner', ?, ?) ON CONFLICT(entity_id) DO UPDATE SET status=excluded.status, source='scanner', observed_at=excluded.observed_at, evidence_json=excluded.evidence_json, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
                .bind(format!("scan-install-{}", entity.id)).bind(&entity.id).bind(status.as_str())
                .bind(report.scanned_at_unix_ms.to_string()).bind(evidence)
                .execute(&mut **tx).await.map_err(|_| "Could not save Codex scan state.".to_string())?;
        }
        let report_json = serde_json::to_string(report)
            .map_err(|_| "Scanner result is invalid and was not saved.".to_string())?;
        sqlx::query("INSERT INTO codex_scan_state(id, scanned_at_unix_ms, report_json) VALUES ('latest', ?, ?) ON CONFLICT(id) DO UPDATE SET scanned_at_unix_ms=excluded.scanned_at_unix_ms, report_json=excluded.report_json, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
            .bind(report.scanned_at_unix_ms.min(i64::MAX as u64) as i64)
            .bind(report_json)
            .execute(&mut **tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        crate::capability_identity::sync_in(tx, report).await?;
        Ok(())
    }

    pub async fn load_codex_scan(
        &self,
    ) -> Result<Option<crate::codex_scanner::CodexScanReport>, String> {
        let report_json: Option<String> =
            sqlx::query_scalar("SELECT report_json FROM codex_scan_state WHERE id = 'latest'")
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| DATABASE_ERROR.to_string())?;
        let mut report: Option<crate::codex_scanner::CodexScanReport> = report_json
            .map(|json| {
                serde_json::from_str(&json).map_err(|_| {
                    "Saved Codex scan report is invalid; existing data was retained.".to_string()
                })
            })
            .transpose()?;
        if let Some(identity) = report.as_mut().and_then(|r| r.identity.as_mut()) {
            let scopes = identity.context.scopes();
            identity.history = self.identity_history(&scopes).await?;
        }
        Ok(report)
    }

    pub async fn list_capabilities(&self) -> Result<Vec<CapabilityRecord>, String> {
        sqlx::query_as::<_, CapabilityRecord>(
            "SELECT e.id, e.kind, e.name, e.summary, e.source_type, e.source_owner, e.source_repo, e.source_ref, e.source_url, e.license_spdx, e.metadata_json, c.id AS primary_category_id, c.name AS primary_category_name FROM entities e LEFT JOIN entity_categories ec ON ec.entity_id = e.id AND ec.role = 'primary' LEFT JOIN categories c ON c.id = ec.category_id ORDER BY e.name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn integrity_check(&self) -> Result<(), String> {
        let result: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        if result != "ok" {
            return Err(
                "Local database integrity check failed. No automatic repair was attempted.".into(),
            );
        }
        let broken: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| DATABASE_ERROR.to_string())?;
        if broken {
            return Err("Local database foreign key check failed.".into());
        }
        Ok(())
    }

    pub async fn create_verified_backup(&self) -> Result<String, String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "Application data directory is unavailable.".to_string())?;
        let backup_dir = parent.join("backups");
        std::fs::create_dir_all(&backup_dir)
            .map_err(|_| "Could not create the application backup directory.".to_string())?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System clock is invalid; backup was not created.".to_string())?
            .as_nanos();
        let backup_path = backup_dir.join(format!("skillverse-{timestamp}.sqlite3"));
        let backup_name = backup_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Could not name the database backup.".to_string())?
            .to_string();

        let mut source = self
            .pool
            .acquire()
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        let source_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *source)
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        sqlx::query("VACUUM INTO ?")
            .bind(backup_path.to_string_lossy().into_owned())
            .execute(&mut *source)
            .await
            .map_err(|_| "SQLite could not create a consistent database snapshot.".to_string())?;
        drop(source);

        let verify_options = SqliteConnectOptions::new()
            .filename(&backup_path)
            .read_only(true)
            .foreign_keys(true);
        let verify_pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(verify_options)
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = std::fs::remove_file(&backup_path);
                return Err("Database backup could not be opened for verification.".into());
            }
        };
        let verified: Result<(), String> = async {
            let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
                .fetch_one(&verify_pool)
                .await
                .map_err(|_| "Backup verification failed.".to_string())?;
            let version: i64 = sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&verify_pool)
                .await
                .map_err(|_| "Backup version verification failed.".to_string())?;
            let broken: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)")
                    .fetch_one(&verify_pool)
                    .await
                    .map_err(|_| "Backup foreign key verification failed.".to_string())?;
            if integrity != "ok"
                || broken
                || version != source_version
                || !(6..=13).contains(&version)
            {
                return Err("Backup verification failed; the snapshot was not accepted.".into());
            }
            Ok(())
        }
        .await;
        verify_pool.close().await;
        if let Err(error) = verified {
            let _ = std::fs::remove_file(&backup_path);
            return Err(error);
        }
        Ok(backup_name)
    }

    pub async fn get_capability(&self, id: &str) -> Result<Option<CapabilityRecord>, String> {
        validate_text(id, "id", 180)?;
        sqlx::query_as::<_, CapabilityRecord>(
            "SELECT e.id, e.kind, e.name, e.summary, e.source_type, e.source_owner, e.source_repo, e.source_ref, e.source_url, e.license_spdx, e.metadata_json, c.id AS primary_category_id, c.name AS primary_category_name FROM entities e LEFT JOIN entity_categories ec ON ec.entity_id = e.id AND ec.role = 'primary' LEFT JOIN categories c ON c.id = ec.category_id WHERE e.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn upsert_capability(
        &self,
        input: CapabilityInput,
    ) -> Result<CapabilityRecord, String> {
        validate_capability(&input)?;
        let metadata = serde_json::to_string(&redact_sensitive(input.metadata))
            .map_err(|_| "Capability metadata is invalid.".to_string())?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        sqlx::query(
            "INSERT INTO entities (id, kind, name, summary, source_type, source_owner, source_repo, source_ref, source_url, license_spdx, metadata_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, name=excluded.name, summary=excluded.summary, source_type=excluded.source_type, source_owner=excluded.source_owner, source_repo=excluded.source_repo, source_ref=excluded.source_ref, source_url=excluded.source_url, license_spdx=excluded.license_spdx, metadata_json=json_patch(entities.metadata_json,excluded.metadata_json), updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&input.id).bind(&input.kind).bind(&input.name).bind(&input.summary)
        .bind(&input.source_type).bind(&input.source_owner).bind(&input.source_repo)
        .bind(&input.source_ref).bind(&input.source_url).bind(&input.license_spdx).bind(metadata)
        .execute(&mut *tx).await.map_err(|_| "Capability conflicts with existing catalog data or violates the data contract.".to_string())?;
        if let Some(category_id) = input.primary_category_id.as_deref() {
            validate_text(category_id, "category id", 180)?;
            sqlx::query("DELETE FROM entity_categories WHERE entity_id = ? AND role = 'primary'")
                .bind(&input.id)
                .execute(&mut *tx)
                .await
                .map_err(|_| DATABASE_ERROR.to_string())?;
            sqlx::query("INSERT INTO entity_categories(entity_id, category_id, role, assigned_by) VALUES (?, ?, 'primary', 'manual')")
                .bind(&input.id).bind(category_id).execute(&mut *tx).await
                .map_err(|_| "Primary category does not exist or violates the data contract.".to_string())?;
        }
        let graph_id = input.id.clone();
        sqlx::query("INSERT INTO graph_nodes(id, kind, entity_id) VALUES (?, 'entity', ?) ON CONFLICT(id) DO NOTHING")
            .bind(graph_id).bind(&input.id).execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
        tx.commit().await.map_err(|_| DATABASE_ERROR.to_string())?;
        self.get_capability(&input.id)
            .await?
            .ok_or_else(|| DATABASE_ERROR.into())
    }

    pub async fn list_domains(&self) -> Result<Vec<DomainRecord>, String> {
        sqlx::query_as::<_, DomainRecord>(
            "SELECT id, parent_id, slug, name, origin FROM categories WHERE parent_id IS NULL ORDER BY name COLLATE NOCASE",
        )
        .fetch_all(&self.pool).await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn get_capability_relations(&self, id: &str) -> Result<Vec<RelationRecord>, String> {
        validate_text(id, "id", 180)?;
        let node_id = id.to_string();
        sqlx::query_as::<_, RelationRecord>(
            "SELECT id, source_node_id, target_node_id, relation FROM graph_edges WHERE source_node_id = ? OR target_node_id = ? ORDER BY relation, id",
        )
        .bind(&node_id).bind(&node_id).fetch_all(&self.pool).await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn save_graph_layout(&self, input: GraphLayoutInput) -> Result<(), String> {
        validate_text(&input.profile, "layout profile", 100)?;
        if input.positions.len() > 2000 {
            return Err("A layout update may contain at most 2,000 nodes.".into());
        }
        let mut seen = std::collections::HashSet::new();
        for position in &input.positions {
            validate_text(&position.id, "graph node id", 240)?;
            if !seen.insert(&position.id)
                || position.x.is_some_and(|x| !x.is_finite())
                || position.y.is_some_and(|y| !y.is_finite())
            {
                return Err(
                    "Graph layout contains duplicate nodes or non-finite coordinates.".into(),
                );
            }
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| DATABASE_ERROR.to_string())?;
        for position in input.positions {
            let result = sqlx::query("UPDATE graph_nodes SET position_x = ?, position_y = ?, pinned = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND layout_profile = ?")
                .bind(position.x).bind(position.y).bind(position.pinned).bind(position.id).bind(&input.profile)
                .execute(&mut *tx).await.map_err(|_| DATABASE_ERROR.to_string())?;
            if result.rows_affected() != 1 {
                return Err("Layout references a node that does not exist in this profile.".into());
            }
        }
        tx.commit().await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn load_graph_layout(&self, profile: &str) -> Result<Vec<GraphPosition>, String> {
        validate_text(profile, "layout profile", 100)?;
        sqlx::query_as::<_, GraphPosition>("SELECT id, position_x AS x, position_y AS y, pinned FROM graph_nodes WHERE layout_profile = ? ORDER BY id")
            .bind(profile).fetch_all(&self.pool).await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn get_installation_status(
        &self,
        id: &str,
    ) -> Result<Option<InstallationRecord>, String> {
        validate_text(id, "entity id", 180)?;
        sqlx::query_as::<_, InstallationRecord>("SELECT id, entity_id, status, source, version, observed_at, evidence_json FROM installations WHERE entity_id = ?")
            .bind(id).fetch_optional(&self.pool).await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn update_installation_status(&self, input: InstallationInput) -> Result<(), String> {
        validate_text(&input.id, "installation id", 180)?;
        validate_text(&input.entity_id, "entity id", 180)?;
        validate_optional(input.source.as_deref(), "installation source", 30)?;
        validate_optional(input.version.as_deref(), "version", 200)?;
        validate_optional(input.observed_at.as_deref(), "observed timestamp", 80)?;
        let evidence = serde_json::to_string(&redact_sensitive(input.evidence))
            .map_err(|_| "Installation evidence is invalid.".to_string())?;
        sqlx::query("INSERT INTO installations(id, entity_id, status, source, version, observed_at, evidence_json) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(entity_id) DO UPDATE SET status=excluded.status, source=excluded.source, version=excluded.version, observed_at=excluded.observed_at, evidence_json=excluded.evidence_json, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
            .bind(input.id).bind(input.entity_id).bind(input.status.as_str()).bind(input.source).bind(input.version).bind(input.observed_at).bind(evidence)
            .execute(&self.pool).await.map_err(|_| "Installation update references an unknown capability or violates the state contract.".into()).map(|_| ())
    }

    pub async fn get_learning_progress(&self, id: &str) -> Result<Option<LearningRecord>, String> {
        validate_text(id, "entity id", 180)?;
        sqlx::query_as::<_, LearningRecord>("SELECT id, entity_id, status, evidence_json, updated_at FROM learning_records WHERE entity_id = ?")
            .bind(id).fetch_optional(&self.pool).await.map_err(|_| DATABASE_ERROR.into())
    }

    pub async fn update_learning_progress(
        &self,
        input: LearningInput,
    ) -> Result<LearningRecord, String> {
        validate_text(&input.id, "learning record id", 180)?;
        validate_text(&input.entity_id, "entity id", 180)?;
        if matches!(input.status, LearningStatus::Verified) {
            return Err(
                "VERIFIED learning progress requires independent evidence verification.".into(),
            );
        }
        if !input.evidence.is_array() {
            return Err("Learning evidence must be a list of self-reported items.".into());
        }
        let evidence = serde_json::to_string(&redact_sensitive(input.evidence))
            .map_err(|_| "Learning evidence is invalid.".to_string())?;
        if evidence.len() > 16 * 1024 {
            return Err("Self-reported learning evidence is too large.".into());
        }
        sqlx::query("INSERT INTO learning_records(id, entity_id, status, evidence_json) VALUES (?, ?, ?, ?) ON CONFLICT(entity_id) DO UPDATE SET status=excluded.status, evidence_json=excluded.evidence_json, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE learning_records.status<>'VERIFIED'")
            .bind(&input.id).bind(&input.entity_id).bind(input.status.as_str()).bind(evidence)
            .execute(&self.pool).await.map_err(|_| String::from("Learning update references an unknown capability or violates the data contract."))?;
        self.get_learning_progress(&input.entity_id)
            .await?
            .ok_or_else(|| DATABASE_ERROR.into())
    }

    pub async fn append_audit_event(&self, input: AuditEventInput) -> Result<(), String> {
        validate_text(&input.id, "audit event id", 180)?;
        validate_text(&input.action, "audit action", 120)?;
        validate_text(&input.target_type, "audit target type", 80)?;
        validate_text(&input.target_id, "audit target id", 180)?;
        if !matches!(
            input.outcome.as_str(),
            "planned" | "succeeded" | "failed" | "rolled_back"
        ) {
            return Err("Audit outcome is not supported.".into());
        }
        let before = json_option_to_string(input.before)?;
        let after = json_option_to_string(input.after)?;
        sqlx::query("INSERT INTO audit_events(id, action, target_type, target_id, outcome, before_json, after_json) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(input.id).bind(input.action).bind(input.target_type).bind(input.target_id).bind(input.outcome).bind(before).bind(after)
            .execute(&self.pool).await.map_err(|_| "Audit event could not be appended.".into()).map(|_| ())
    }
}

fn validate_capability(input: &CapabilityInput) -> Result<(), String> {
    validate_text(&input.id, "capability id", 180)?;
    validate_text(&input.name, "capability name", 240)?;
    validate_optional(Some(&input.summary), "summary", 4000)?;
    if !matches!(
        input.kind.as_str(),
        "skill" | "mcp" | "plugin" | "capability"
    ) {
        return Err("Capability kind is not supported.".into());
    }
    if let Some(source_type) = input.source_type.as_deref() {
        if !matches!(source_type, "github" | "local" | "manual") {
            return Err("Capability source type is not supported.".into());
        }
    }
    for (value, label, max) in [
        (input.source_owner.as_deref(), "source owner", 100),
        (input.source_repo.as_deref(), "source repository", 100),
        (input.source_ref.as_deref(), "source ref", 200),
        (input.source_url.as_deref(), "source URL", 2048),
        (input.license_spdx.as_deref(), "license", 80),
    ] {
        validate_optional(value, label, max)?;
    }
    if input.metadata.to_string().len() > 64_000 {
        return Err("Capability metadata exceeds 64 KB.".into());
    }
    if input.source_type.as_deref() == Some("github")
        && (input.source_owner.is_none() || input.source_repo.is_none())
    {
        return Err("GitHub capabilities require an owner and repository.".into());
    }
    Ok(())
}

fn validate_text(value: &str, label: &str, max: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max || value.contains('\0') {
        return Err(format!("{label} must contain 1 to {max} valid characters."));
    }
    Ok(())
}

fn validate_optional(value: Option<&str>, label: &str, max: usize) -> Result<(), String> {
    if let Some(value) = value {
        if !value.is_empty() {
            validate_text(value, label, max)?;
        }
    }
    Ok(())
}

fn json_option_to_string(value: Option<serde_json::Value>) -> Result<Option<String>, String> {
    value
        .map(|value| {
            serde_json::to_string(&redact_sensitive(value))
                .map_err(|_| "Audit payload is invalid.".into())
        })
        .transpose()
}

fn redact_sensitive(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(entries) => serde_json::Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase().replace('-', "").replace('_', "");
                    let sensitive = [
                        "apikey",
                        "token",
                        "authorization",
                        "password",
                        "secret",
                        "credential",
                    ]
                    .iter()
                    .any(|marker| normalized.contains(marker));
                    (
                        key,
                        if sensitive {
                            serde_json::Value::String("[REDACTED]".into())
                        } else {
                            redact_sensitive(value)
                        },
                    )
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redact_sensitive).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_database(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("skillverse-{label}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("skillverse.sqlite3")
    }

    #[tokio::test]
    async fn patched_sqlite_recovers_committed_wal_after_process_termination() {
        let path = temporary_database("patched-sqlite");
        let directory = path.parent().unwrap();
        let database = Database::open_desktop(path.clone()).await.unwrap();
        let (version, source): (String, String) =
            sqlx::query_as("SELECT sqlite_version(), sqlite_source_id()")
                .fetch_one(&database.pool)
                .await
                .unwrap();
        assert_eq!(version, "3.54.1");
        assert!(
            source.contains("555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76")
        );
        let cipher: Option<String> = sqlx::query_scalar("PRAGMA cipher_version")
            .fetch_optional(&database.pool)
            .await
            .unwrap();
        assert_eq!(cipher, None);
        database.pool.close().await;

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "db::tests::patched_sqlite_wal_worker"])
            .env("SKILLVERSE_TEST_SQLITE_WAL_DIRECTORY", directory)
            .spawn()
            .unwrap();
        let ready = directory.join("wal-ready");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !ready.is_file() && std::time::Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let terminated = child.kill();
        child.wait().unwrap();
        assert!(ready.is_file(), "worker did not finish writing the WAL");
        terminated.unwrap();
        assert!(
            directory
                .join("skillverse.sqlite3-wal")
                .metadata()
                .unwrap()
                .len()
                > 32
        );

        let recovered = Database::open_desktop(path.clone()).await.unwrap();
        let rows: Vec<i64> = sqlx::query_scalar("SELECT value FROM wal_fixture ORDER BY value")
            .fetch_all(&recovered.pool)
            .await
            .unwrap();
        assert_eq!(rows, [1]);
        recovered.integrity_check().await.unwrap();
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_optional(&recovered.pool)
                .await
                .unwrap()
                .is_none()
        );
        recovered.pool.close().await;
        remove_test_directory(directory.to_path_buf());
    }

    #[tokio::test]
    async fn patched_sqlite_wal_worker() {
        let Some(directory) = std::env::var_os("SKILLVERSE_TEST_SQLITE_WAL_DIRECTORY") else {
            return;
        };
        let directory = PathBuf::from(directory).canonicalize().unwrap();
        assert_eq!(
            directory.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("skillverse-patched-sqlite-")
        );
        let database = Database::open_desktop(directory.join("skillverse.sqlite3"))
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_autocheckpoint=0")
            .execute(&database.pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE wal_fixture(value INTEGER); INSERT INTO wal_fixture VALUES(1);",
        )
        .execute(&database.pool)
        .await
        .unwrap();
        let mut pending = database.pool.begin().await.unwrap();
        sqlx::query("INSERT INTO wal_fixture VALUES(2)")
            .execute(&mut *pending)
            .await
            .unwrap();
        std::fs::write(directory.join("wal-ready"), []).unwrap();
        std::future::pending::<()>().await;
    }

    #[tokio::test]
    async fn desktop_bootstraps_empty_profiles_without_migrating_versioned_profiles() {
        let fresh = temporary_database("desktop-bootstrap");
        for _ in 0..2 {
            let db = Database::open_desktop(fresh.clone()).await.unwrap();
            let marker: i64 = sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&db.pool)
                .await
                .unwrap();
            assert_eq!(marker, 13);
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM schema_migrations")
                    .fetch_one(&db.pool)
                    .await
                    .unwrap(),
                13
            );
            db.integrity_check().await.unwrap();
            db.pool.close().await;
        }
        remove_test_directory(fresh.parent().unwrap().to_path_buf());
        let existing = temporary_database("desktop-existing");
        Database::open(existing.clone())
            .await
            .unwrap()
            .pool
            .close()
            .await;
        let db = Database::open_desktop(existing.clone()).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            6
        );
        db.pool.close().await;
        remove_test_directory(existing.parent().unwrap().to_path_buf());
    }

    fn remove_test_directory(directory: PathBuf) {
        let resolved = directory.canonicalize().unwrap();
        let temporary_root = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(resolved.parent(), Some(temporary_root.as_path()));
        assert!(
            resolved
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("skillverse-")
        );
        // Windows can briefly retain a SQLite/antivirus handle after close.
        // Retry only this test's verified direct child of the temporary folder.
        for attempt in 0..60 {
            match std::fs::remove_dir_all(&resolved) {
                Ok(()) => return,
                Err(error) if attempt < 59 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                    std::thread::sleep(std::time::Duration::from_millis(50))
                }
                Err(error) => panic!("Could not remove the test database directory: {error}"),
            }
        }
    }

    fn capability(id: &str) -> CapabilityInput {
        CapabilityInput {
            id: id.into(),
            kind: "skill".into(),
            name: "Test skill".into(),
            summary: "Local fixture".into(),
            source_type: Some("manual".into()),
            source_owner: None,
            source_repo: None,
            source_ref: None,
            source_url: None,
            license_spdx: None,
            metadata: serde_json::json!({"safe": true}),
            primary_category_id: Some("Engineering".into()),
        }
    }

    #[test]
    fn discovery_catalog_survives_reopen_without_inventing_installations() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("discovery-catalog");
            let parent = path.parent().unwrap().to_path_buf();
            let database = Database::open(path.clone()).await.unwrap();
            let expected: serde_json::Value = serde_json::from_str(DISCOVERY_CATALOG).unwrap();
            let entries = expected["items"].as_array().unwrap();
            assert!(entries.len() > 150);
            for item in entries {
                let id = item["id"].as_str().unwrap();
                let stored = database.get_capability(id).await.unwrap().unwrap();
                assert_eq!(stored.source_ref.as_deref(), item["sourceRef"].as_str());
                assert_eq!(stored.kind, item["kind"].as_str().unwrap());
                assert!(
                    database
                        .get_installation_status(id)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
            let total = database.list_capabilities().await.unwrap().len();
            database.pool.close().await;
            let reopened = Database::open(path).await.unwrap();
            assert_eq!(reopened.list_capabilities().await.unwrap().len(), total);
            reopened.integrity_check().await.unwrap();
            reopened.pool.close().await;
            drop(reopened);
            drop(database);
            remove_test_directory(parent);
        });
    }

    #[test]
    fn migration_reopen_repository_layout_learning_and_verified_backup() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("lifecycle");
            let parent = path.parent().unwrap().to_path_buf();
            let database = Database::open(path.clone()).await.unwrap();
            database.integrity_check().await.unwrap();
            assert_eq!(database.list_domains().await.unwrap().len(), 4);

            let saved = database
                .upsert_capability(capability("test-skill"))
                .await
                .unwrap();
            assert_eq!(saved.primary_category_name.as_deref(), Some("Engineering"));
            assert_eq!(
                database
                    .get_capability("test-skill")
                    .await
                    .unwrap()
                    .unwrap()
                    .name,
                "Test skill"
            );

            database
                .save_graph_layout(GraphLayoutInput {
                    profile: "world-tree".into(),
                    positions: vec![GraphPosition {
                        id: "test-skill".into(),
                        x: Some(123.0),
                        y: Some(456.0),
                        pinned: true,
                    }],
                })
                .await
                .unwrap();
            assert_eq!(
                database
                    .load_graph_layout("world-tree")
                    .await
                    .unwrap()
                    .iter()
                    .find(|row| row.id == "test-skill")
                    .unwrap()
                    .x,
                Some(123.0)
            );

            let learning = database
                .update_learning_progress(LearningInput {
                    id: "learning-test-skill".into(),
                    entity_id: "test-skill".into(),
                    status: LearningStatus::Exploring,
                    evidence: serde_json::json!([]),
                })
                .await
                .unwrap();
            assert_eq!(learning.status, "EXPLORING");
            let invalid_verified = database
                .update_learning_progress(LearningInput {
                    id: "learning-test-skill".into(),
                    entity_id: "test-skill".into(),
                    status: LearningStatus::Verified,
                    evidence: serde_json::json!([{"type":"forged","verified":true}]),
                })
                .await;
            assert!(invalid_verified.is_err());

            let backup_name = database.create_verified_backup().await.unwrap();
            assert!(parent.join("backups").join(backup_name).is_file());
            database.pool.close().await;

            let reopened = Database::open(path).await.unwrap();
            assert_eq!(
                reopened
                    .get_learning_progress("test-skill")
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "EXPLORING"
            );
            assert_eq!(
                reopened
                    .get_capability("test-skill")
                    .await
                    .unwrap()
                    .unwrap()
                    .name,
                "Test skill"
            );
            reopened.pool.close().await;
            drop(reopened);
            drop(database);
            remove_test_directory(parent);
        });
    }

    #[test]
    fn upgrades_version_one_database_without_losing_existing_rows() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("upgrade-v1");
            let parent = path.parent().unwrap().to_path_buf();
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&path)
                        .create_if_missing(true),
                )
                .await
                .unwrap();
            sqlx::raw_sql(MIGRATION_0001).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO entities(id, kind, name, summary) VALUES ('keep-v1', 'skill', 'Existing v1', 'preserved')")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;

            let upgraded = Database::open(path).await.unwrap();
            let version: i64 = sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&upgraded.pool)
                .await
                .unwrap();
            assert_eq!(version, 6);
            assert_eq!(
                upgraded
                    .get_capability("keep-v1")
                    .await
                    .unwrap()
                    .unwrap()
                    .name,
                "Existing v1"
            );
            upgraded.pool.close().await;
            drop(upgraded);
            remove_test_directory(parent);
        });
    }

    #[test]
    fn rejects_unversioned_existing_database_without_overwriting_it() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("unversioned");
            let parent = path.parent().unwrap().to_path_buf();
            let options = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();
            sqlx::query("CREATE TABLE keep_me (value TEXT NOT NULL)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO keep_me(value) VALUES ('preserved')")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;

            assert!(Database::open(path.clone()).await.is_err());
            let verify = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(SqliteConnectOptions::new().filename(&path))
                .await
                .unwrap();
            let value: String = sqlx::query_scalar("SELECT value FROM keep_me")
                .fetch_one(&verify)
                .await
                .unwrap();
            assert_eq!(value, "preserved");
            verify.close().await;
            drop(verify);
            drop(pool);
            remove_test_directory(parent);
        });
    }

    #[test]
    fn validates_input_and_redacts_secret_shaped_json_fields() {
        assert!(validate_capability(&capability("valid")).is_ok());
        let mut invalid = capability(" ");
        assert!(validate_capability(&invalid).is_err());
        invalid.id = "valid".into();
        invalid.metadata = serde_json::json!({"safe": "ok", "access_token": "do-not-store"});
        let stored = redact_sensitive(invalid.metadata);
        assert_eq!(stored["access_token"], "[REDACTED]");
        assert_eq!(stored["safe"], "ok");
    }

    #[test]
    fn persists_scan_findings_and_marks_missing_scanner_owned_items_stale() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("scan-persistence");
            let parent = path.parent().unwrap().to_path_buf();
            let database = Database::open(path).await.unwrap();
            let report = crate::codex_scanner::CodexScanReport {
                codex_home_found: true,
                cli_found_on_path: true,
                scanned_at_unix_ms: 1_800_000_000_000,
                skills: Vec::new(),
                mcp_servers: Vec::new(),
                plugins: Vec::new(),
                warnings: Vec::new(),
                identity: None,
                entities: vec![crate::codex_scanner::ScannedEntity {
                    id: "skill:local:fixture".into(),
                    kind: "skill".into(),
                    name: "Fixture skill".into(),
                    summary: "Fixture description".into(),
                    status: InstallationStatus::Installed,
                    evidence: serde_json::json!({"scope": "user", "runtimeInvoked": false}),
                }],
            };
            database.persist_codex_scan(&report).await.unwrap();
            let restored = database.load_codex_scan().await.unwrap().unwrap();
            assert_eq!(restored.scanned_at_unix_ms, report.scanned_at_unix_ms);
            assert!(restored.codex_home_found);
            assert_eq!(restored.skills.len(), report.skills.len());
            assert_eq!(
                database
                    .get_capability("skill:local:fixture")
                    .await
                    .unwrap()
                    .unwrap()
                    .name,
                "Fixture skill"
            );
            assert_eq!(
                database
                    .get_installation_status("skill:local:fixture")
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "INSTALLED"
            );

            let empty_report = crate::codex_scanner::CodexScanReport {
                entities: Vec::new(),
                scanned_at_unix_ms: 1_800_000_000_001,
                ..report
            };
            database.persist_codex_scan(&empty_report).await.unwrap();
            let restored = database.load_codex_scan().await.unwrap().unwrap();
            assert_eq!(restored.scanned_at_unix_ms, empty_report.scanned_at_unix_ms);
            assert!(restored.skills.is_empty());
            assert_eq!(
                database
                    .get_installation_status("skill:local:fixture")
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                "NOT_INSTALLED"
            );
            database.pool.close().await;
            drop(database);
            remove_test_directory(parent);
        });
    }
    #[test]
    fn migration_six_failure_rolls_back_schema_and_preserves_existing_rows() {
        tauri::async_runtime::block_on(async {
            let path = temporary_database("migration-six-rollback");
            let parent = path.parent().unwrap().to_path_buf();
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&path)
                        .create_if_missing(true),
                )
                .await
                .unwrap();
            for migration in [
                MIGRATION_0001,
                MIGRATION_0002,
                MIGRATION_0003,
                MIGRATION_0004,
                MIGRATION_0005,
            ] {
                sqlx::raw_sql(migration).execute(&pool).await.unwrap();
            }
            sqlx::query("INSERT INTO entities(id,kind,name,summary) VALUES('preserve-six','skill','Preserved before schema six','do not change')").execute(&pool).await.unwrap();
            sqlx::query("CREATE TRIGGER reject_six BEFORE INSERT ON schema_migrations WHEN NEW.version=6 BEGIN SELECT RAISE(ABORT,'isolated migration failure'); END").execute(&pool).await.unwrap();
            pool.close().await;
            assert!(
                Database::open(path.clone())
                    .await
                    .err()
                    .unwrap()
                    .contains("0006")
            );
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(SqliteConnectOptions::new().filename(&path))
                .await
                .unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                5
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM sqlite_master WHERE name='installation_operations'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                0
            );
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT summary FROM entities WHERE id='preserve-six'"
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                "do not change"
            );
            let backups = std::fs::read_dir(parent.join("backups"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("before-schema-6-")
                })
                .count();
            assert_eq!(backups, 1);
            sqlx::query("DROP TRIGGER reject_six")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
            let db = Database::open(path).await.unwrap();
            assert_eq!(
                db.get_capability("preserve-six")
                    .await
                    .unwrap()
                    .unwrap()
                    .summary,
                "do not change"
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("PRAGMA user_version")
                    .fetch_one(&db.pool)
                    .await
                    .unwrap(),
                6
            );
            db.pool.close().await;
            drop(db);
            drop(pool);
            remove_test_directory(parent);
        });
    }
}
