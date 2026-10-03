CREATE TABLE speakers (
    id TEXT PRIMARY KEY NOT NULL,
    analysis_revision_id TEXT NOT NULL REFERENCES analysis_revisions(id) ON DELETE RESTRICT,
    lineage_id TEXT NOT NULL,
    label TEXT NOT NULL,
    display_name TEXT,
    input_scope TEXT NOT NULL,
    identification_state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (analysis_revision_id, id),
    UNIQUE (analysis_revision_id, lineage_id)
);

CREATE TABLE speaker_segments (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    speaker_id TEXT,
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    overlap_group_id TEXT,
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    FOREIGN KEY (meeting_id, analysis_revision_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id),
    FOREIGN KEY (analysis_revision_id, speaker_id) REFERENCES speakers(analysis_revision_id, id)
);
CREATE INDEX speaker_segments_time ON speaker_segments(analysis_revision_id, start_us);

CREATE TABLE utterances (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    track_id TEXT NOT NULL,
    lineage_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    recognized_text TEXT NOT NULL,
    effective_text TEXT NOT NULL,
    attribution_state TEXT NOT NULL CHECK (attribution_state IN ('assigned', 'overlap', 'unknown')),
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    UNIQUE (analysis_revision_id, id),
    UNIQUE (analysis_revision_id, lineage_id),
    UNIQUE (analysis_revision_id, sequence),
    FOREIGN KEY (meeting_id, analysis_revision_id) REFERENCES analysis_revisions(meeting_id, id),
    FOREIGN KEY (meeting_id, track_id) REFERENCES audio_tracks(meeting_id, id)
);
CREATE INDEX utterances_time ON utterances(analysis_revision_id, start_us);

CREATE TABLE utterance_speakers (
    utterance_id TEXT NOT NULL,
    speaker_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('primary', 'overlap')),
    assignment_source TEXT NOT NULL CHECK (assignment_source IN ('inferred', 'manual')),
    PRIMARY KEY (utterance_id, speaker_id),
    FOREIGN KEY (analysis_revision_id, utterance_id) REFERENCES utterances(analysis_revision_id, id),
    FOREIGN KEY (analysis_revision_id, speaker_id) REFERENCES speakers(analysis_revision_id, id)
);

CREATE TABLE minutes_items (
    id TEXT PRIMARY KEY NOT NULL,
    minutes_revision_id TEXT NOT NULL REFERENCES minutes_revisions(id) ON DELETE RESTRICT,
    analysis_revision_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('summary', 'topic', 'decision', 'open_issue', 'todo')),
    order_no INTEGER NOT NULL CHECK (order_no >= 0),
    body TEXT NOT NULL,
    assignee_name TEXT,
    assignee_speaker_id TEXT,
    due_date TEXT CHECK (due_date IS NULL OR due_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    due_text TEXT,
    source_type TEXT NOT NULL,
    UNIQUE (minutes_revision_id, id),
    UNIQUE (minutes_revision_id, order_no),
    FOREIGN KEY (analysis_revision_id, assignee_speaker_id) REFERENCES speakers(analysis_revision_id, id)
);

CREATE TABLE evidence_links (
    id TEXT PRIMARY KEY NOT NULL,
    minutes_revision_id TEXT NOT NULL,
    minutes_item_id TEXT NOT NULL,
    analysis_revision_id TEXT NOT NULL,
    utterance_id TEXT NOT NULL,
    quote_text TEXT,
    start_us INTEGER NOT NULL CHECK (start_us >= 0),
    end_us INTEGER NOT NULL CHECK (end_us > start_us),
    source_type TEXT NOT NULL,
    verification_state TEXT NOT NULL,
    FOREIGN KEY (minutes_revision_id, minutes_item_id) REFERENCES minutes_items(minutes_revision_id, id),
    FOREIGN KEY (analysis_revision_id, utterance_id) REFERENCES utterances(analysis_revision_id, id)
);
CREATE INDEX evidence_links_utterance ON evidence_links(utterance_id);

CREATE TRIGGER minutes_item_input BEFORE INSERT ON minutes_items
WHEN NEW.analysis_revision_id IS NOT (SELECT input_analysis_id FROM minutes_revisions WHERE id = NEW.minutes_revision_id)
BEGIN SELECT RAISE(ABORT, 'minutes item has wrong analysis'); END;
CREATE TRIGGER minutes_item_input_update BEFORE UPDATE ON minutes_items
WHEN NEW.analysis_revision_id IS NOT (SELECT input_analysis_id FROM minutes_revisions WHERE id = NEW.minutes_revision_id)
BEGIN SELECT RAISE(ABORT, 'minutes item has wrong analysis'); END;

CREATE TRIGGER evidence_input BEFORE INSERT ON evidence_links
WHEN NEW.analysis_revision_id IS NOT (SELECT input_analysis_id FROM minutes_revisions WHERE id = NEW.minutes_revision_id)
  OR NOT EXISTS (SELECT 1 FROM utterances WHERE id = NEW.utterance_id AND analysis_revision_id = NEW.analysis_revision_id
      AND start_us <= NEW.start_us AND end_us >= NEW.end_us)
BEGIN SELECT RAISE(ABORT, 'evidence is outside the input utterance'); END;
CREATE TRIGGER evidence_input_update BEFORE UPDATE ON evidence_links
WHEN NEW.analysis_revision_id IS NOT (SELECT input_analysis_id FROM minutes_revisions WHERE id = NEW.minutes_revision_id)
  OR NOT EXISTS (SELECT 1 FROM utterances WHERE id = NEW.utterance_id AND analysis_revision_id = NEW.analysis_revision_id
      AND start_us <= NEW.start_us AND end_us >= NEW.end_us)
BEGIN SELECT RAISE(ABORT, 'evidence is outside the input utterance'); END;

CREATE TRIGGER speakers_frozen_insert BEFORE INSERT ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER speakers_frozen_update BEFORE UPDATE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER speakers_frozen_delete BEFORE DELETE ON speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER segments_frozen_insert BEFORE INSERT ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER segments_frozen_update BEFORE UPDATE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER segments_frozen_delete BEFORE DELETE ON speaker_segments
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER utterances_frozen_insert BEFORE INSERT ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER utterances_frozen_update BEFORE UPDATE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER utterances_frozen_delete BEFORE DELETE ON utterances
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER assignments_frozen_insert BEFORE INSERT ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER assignments_frozen_update BEFORE UPDATE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
  OR (SELECT publication_state FROM analysis_revisions WHERE id = NEW.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;
CREATE TRIGGER assignments_frozen_delete BEFORE DELETE ON utterance_speakers
WHEN (SELECT publication_state FROM analysis_revisions WHERE id = OLD.analysis_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed analysis content is immutable'); END;

CREATE TRIGGER minutes_items_frozen_insert BEFORE INSERT ON minutes_items
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = NEW.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;
CREATE TRIGGER minutes_items_frozen_update BEFORE UPDATE ON minutes_items
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = OLD.minutes_revision_id) = 'committed'
  OR (SELECT publication_state FROM minutes_revisions WHERE id = NEW.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;
CREATE TRIGGER minutes_items_frozen_delete BEFORE DELETE ON minutes_items
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = OLD.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;

CREATE TRIGGER evidence_frozen_insert BEFORE INSERT ON evidence_links
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = NEW.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;
CREATE TRIGGER evidence_frozen_update BEFORE UPDATE ON evidence_links
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = OLD.minutes_revision_id) = 'committed'
  OR (SELECT publication_state FROM minutes_revisions WHERE id = NEW.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;
CREATE TRIGGER evidence_frozen_delete BEFORE DELETE ON evidence_links
WHEN (SELECT publication_state FROM minutes_revisions WHERE id = OLD.minutes_revision_id) = 'committed'
BEGIN SELECT RAISE(ABORT, 'committed minutes content is immutable'); END;
