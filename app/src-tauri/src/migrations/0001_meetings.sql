CREATE TABLE meetings (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    started_at_utc_ms INTEGER,
    duration_us INTEGER NOT NULL DEFAULT 0 CHECK (duration_us >= 0),
    lifecycle TEXT NOT NULL DEFAULT 'draft' CHECK (lifecycle IN ('draft', 'recording', 'recorded', 'imported', 'archived')),
    next_analysis_no INTEGER NOT NULL DEFAULT 1 CHECK (next_analysis_no > 0),
    next_minutes_no INTEGER NOT NULL DEFAULT 1 CHECK (next_minutes_no > 0),
    lock_version INTEGER NOT NULL DEFAULT 0 CHECK (lock_version >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX meetings_updated_at ON meetings(updated_at DESC, id DESC);
