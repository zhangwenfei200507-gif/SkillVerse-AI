-- A GitHub repository can contain many Skills and Plugins at one revision.
-- Empty paths retain the existing repository-level uniqueness contract.
BEGIN IMMEDIATE;
ALTER TABLE entities ADD COLUMN source_path TEXT NOT NULL DEFAULT '';
DROP INDEX entities_github_source_unique;
CREATE UNIQUE INDEX entities_github_source_unique
    ON entities(lower(source_owner), lower(source_repo), COALESCE(source_ref, ''), source_path)
    WHERE source_type = 'github';
INSERT INTO schema_migrations(version, name) VALUES (4, 'github_component_paths');
PRAGMA user_version = 4;
COMMIT;
