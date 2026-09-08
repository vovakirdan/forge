-- Catalog revisions and publication counters are mutable; graph versions remain immutable.
ALTER TABLE pipelines ADD COLUMN latest_version BIGINT NOT NULL DEFAULT 1
    CHECK (latest_version BETWEEN 1 AND 4294967295);

UPDATE pipelines p SET
    latest_version = COALESCE((SELECT max(v.version) FROM pipeline_versions v WHERE v.pipeline_id=p.id), 1),
    revision = GREATEST(p.revision, 1),
    canonical_snapshot = p.canonical_snapshot || jsonb_build_object(
        'revision', GREATEST(p.revision, 1),
        'latest_version', COALESCE((SELECT max(v.version) FROM pipeline_versions v WHERE v.pipeline_id=p.id), 1)
    );
