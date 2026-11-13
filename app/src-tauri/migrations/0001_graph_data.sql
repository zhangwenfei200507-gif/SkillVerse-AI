-- SkillVerse local graph data, schema version 1.
-- Applied by a native migration runner inside a transaction. Never execute from the UI.
-- The connection must enable PRAGMA foreign_keys = ON before this transaction begins.
BEGIN IMMEDIATE;

CREATE TABLE schema_migrations (
    version         INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    applied_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE categories (
    id              TEXT PRIMARY KEY,
    parent_id       TEXT REFERENCES categories(id) ON DELETE RESTRICT,
    slug            TEXT NOT NULL CHECK (length(trim(slug)) > 0),
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    origin          TEXT NOT NULL DEFAULT 'manual' CHECK (origin IN ('manual', 'seed', 'rule', 'ai_proposal')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE UNIQUE INDEX categories_parent_slug_unique ON categories(COALESCE(parent_id, ''), slug);
CREATE INDEX categories_parent_idx ON categories(parent_id);

CREATE TABLE entities (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL CHECK (kind IN ('skill', 'mcp', 'plugin', 'capability')),
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    summary         TEXT NOT NULL DEFAULT '',
    source_type     TEXT CHECK (source_type IS NULL OR source_type IN ('github', 'local', 'manual')),
    source_owner    TEXT,
    source_repo     TEXT,
    source_ref      TEXT,
    source_url      TEXT,
    license_spdx    TEXT,
    metadata_json   TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((source_owner IS NULL AND source_repo IS NULL) OR
           (source_owner IS NOT NULL AND source_repo IS NOT NULL)),
    CHECK (source_type != 'github' OR (source_owner IS NOT NULL AND source_repo IS NOT NULL))
);
CREATE UNIQUE INDEX entities_github_source_unique
    ON entities(lower(source_owner), lower(source_repo), COALESCE(source_ref, ''))
    WHERE source_type = 'github';
CREATE INDEX entities_kind_name_idx ON entities(kind, name COLLATE NOCASE);
CREATE INDEX entities_source_idx ON entities(source_type, source_owner, source_repo);

CREATE TABLE entity_categories (
    entity_id       TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    category_id     TEXT NOT NULL REFERENCES categories(id) ON DELETE RESTRICT,
    role            TEXT NOT NULL CHECK (role IN ('primary', 'related')),
    assigned_by     TEXT NOT NULL DEFAULT 'manual' CHECK (assigned_by IN ('manual', 'rule', 'ai')),
    confidence      REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (entity_id, category_id)
);
CREATE UNIQUE INDEX entity_one_primary_category ON entity_categories(entity_id) WHERE role = 'primary';
CREATE INDEX entity_categories_category_idx ON entity_categories(category_id, role);

CREATE TABLE graph_nodes (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL CHECK (kind IN ('core', 'category', 'entity')),
    category_id     TEXT REFERENCES categories(id) ON DELETE CASCADE,
    entity_id       TEXT REFERENCES entities(id) ON DELETE CASCADE,
    layout_profile  TEXT NOT NULL DEFAULT 'world-tree',
    position_x      REAL,
    position_y      REAL,
    pinned          INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (
        (kind = 'core' AND category_id IS NULL AND entity_id IS NULL) OR
        (kind = 'category' AND category_id IS NOT NULL AND entity_id IS NULL) OR
        (kind = 'entity' AND category_id IS NULL AND entity_id IS NOT NULL)
    )
);
CREATE UNIQUE INDEX graph_one_core_per_layout ON graph_nodes(layout_profile) WHERE kind = 'core';
CREATE UNIQUE INDEX graph_node_category_unique ON graph_nodes(layout_profile, category_id) WHERE category_id IS NOT NULL;
CREATE UNIQUE INDEX graph_node_entity_unique ON graph_nodes(layout_profile, entity_id) WHERE entity_id IS NOT NULL;
CREATE INDEX graph_nodes_layout_idx ON graph_nodes(layout_profile, kind);

CREATE TABLE graph_edges (
    id              TEXT PRIMARY KEY,
    source_node_id  TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    target_node_id  TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    relation        TEXT NOT NULL CHECK (relation IN (
                        'BELONGS_TO', 'DEPENDS_ON', 'ENABLES', 'OVERLAPS_WITH',
                        'RELATED_TO', 'ALTERNATIVE_TO'
                    )),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (source_node_id != target_node_id),
    UNIQUE (source_node_id, target_node_id, relation)
);
CREATE INDEX graph_edges_target_idx ON graph_edges(target_node_id, relation);

CREATE TABLE installations (
    id              TEXT PRIMARY KEY,
    entity_id       TEXT NOT NULL REFERENCES entities(id) ON DELETE RESTRICT,
    status          TEXT NOT NULL CHECK (status IN ('UNKNOWN', 'NOT_INSTALLED', 'INSTALLED', 'CONFLICT', 'UNSUPPORTED', 'ERROR')),
    source          TEXT CHECK (source IS NULL OR source IN ('scanner', 'app')),
    version         TEXT,
    observed_at     TEXT,
    evidence_json   TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(evidence_json)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (entity_id)
);
CREATE INDEX installations_status_idx ON installations(status, updated_at);

CREATE TABLE learning_records (
    id              TEXT PRIMARY KEY,
    entity_id       TEXT NOT NULL REFERENCES entities(id) ON DELETE RESTRICT,
    status          TEXT NOT NULL CHECK (status IN ('NOT_LEARNED', 'EXPLORING', 'PRACTICING', 'VERIFIED', 'SELF_REPORTED')),
    evidence_json   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence_json)),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (entity_id)
);

CREATE TABLE audit_events (
    id              TEXT PRIMARY KEY,
    action          TEXT NOT NULL CHECK (length(trim(action)) > 0),
    target_type     TEXT NOT NULL,
    target_id       TEXT NOT NULL,
    outcome         TEXT NOT NULL CHECK (outcome IN ('planned', 'succeeded', 'failed', 'rolled_back')),
    before_json     TEXT CHECK (before_json IS NULL OR json_valid(before_json)),
    after_json      TEXT CHECK (after_json IS NULL OR json_valid(after_json)),
    occurred_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX audit_events_target_time_idx ON audit_events(target_type, target_id, occurred_at);

CREATE TRIGGER audit_events_no_update
BEFORE UPDATE ON audit_events BEGIN
    SELECT RAISE(ABORT, 'audit_events are append-only');
END;
CREATE TRIGGER audit_events_no_delete
BEFORE DELETE ON audit_events BEGIN
    SELECT RAISE(ABORT, 'audit_events are append-only');
END;

INSERT INTO schema_migrations(version, name) VALUES (1, 'graph_data');
PRAGMA user_version = 1;
COMMIT;
