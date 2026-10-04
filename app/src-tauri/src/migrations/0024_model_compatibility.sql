-- Device-local runtime trials are never portable availability evidence.
CREATE TABLE model_compatibility_checks (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES model_packages(id),
    review_hash TEXT NOT NULL CHECK(length(review_hash)=64),
    adapter_revision INTEGER NOT NULL CHECK(adapter_revision=1),
    state TEXT NOT NULL CHECK(state IN ('preparing','succeeded','failed','cancelled','interrupted')),
    report_json TEXT CHECK(report_json IS NULL OR (json_valid(report_json) AND length(CAST(report_json AS BLOB))<=65536)),
    report_hash TEXT CHECK(report_hash IS NULL OR length(report_hash)=64),
    error_message TEXT CHECK(error_message IS NULL OR length(CAST(error_message AS BLOB))<=4096),
    created_at INTEGER NOT NULL CHECK(created_at>=0),
    finished_at INTEGER CHECK(finished_at IS NULL OR finished_at>=created_at),
    CHECK((state='preparing' AND finished_at IS NULL AND report_json IS NULL AND report_hash IS NULL AND error_message IS NULL)
       OR (state='succeeded' AND finished_at IS NOT NULL AND report_json IS NOT NULL AND report_hash IS NOT NULL AND error_message IS NULL)
       OR (state IN ('failed','cancelled','interrupted') AND finished_at IS NOT NULL AND report_json IS NULL AND report_hash IS NULL AND error_message IS NOT NULL))
);
CREATE UNIQUE INDEX one_model_compatibility_check ON model_compatibility_checks((1)) WHERE state='preparing';
CREATE INDEX model_compatibility_package ON model_compatibility_checks(package_id);
CREATE TRIGGER model_compatibility_fixed BEFORE UPDATE ON model_compatibility_checks
WHEN OLD.state!='preparing' OR NEW.id!=OLD.id OR NEW.package_id!=OLD.package_id
 OR NEW.review_hash!=OLD.review_hash OR NEW.adapter_revision!=OLD.adapter_revision OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'model trial identity or completed evidence is immutable'); END;
