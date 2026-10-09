-- Immutable historical evidence carried by logical backup/restore. Mutable
-- notice ledgers are never registered as fixed-hash file_assets.
ALTER TABLE recording_sessions ADD COLUMN input_history_snapshot_json TEXT
    CHECK (input_history_snapshot_json IS NULL OR
        (json_valid(input_history_snapshot_json) AND
         length(CAST(input_history_snapshot_json AS BLOB)) <= 525312));

CREATE TRIGGER recording_input_snapshot_owner BEFORE INSERT ON recording_sessions
WHEN NEW.input_history_snapshot_json IS NOT NULL AND
    (NEW.state NOT IN ('saved','interrupted','failed') OR
     json_extract(NEW.input_history_snapshot_json,'$.schemaVersion') IS NOT 1 OR
     json_extract(NEW.input_history_snapshot_json,'$.meetingId') IS NOT NEW.meeting_id OR
     json_extract(NEW.input_history_snapshot_json,'$.sessionId') IS NOT NEW.id)
BEGIN SELECT RAISE(ABORT, 'Input snapshot requires its closed recording owner'); END;

CREATE TRIGGER recording_input_snapshot_binding BEFORE UPDATE OF id,meeting_id,state ON recording_sessions
WHEN NEW.input_history_snapshot_json IS NOT NULL AND
    (NEW.state NOT IN ('saved','interrupted','failed') OR
     json_extract(NEW.input_history_snapshot_json,'$.meetingId') IS NOT NEW.meeting_id OR
     json_extract(NEW.input_history_snapshot_json,'$.sessionId') IS NOT NEW.id)
BEGIN SELECT RAISE(ABORT, 'Input snapshot owner cannot be changed'); END;

CREATE TRIGGER recording_input_snapshot_fixed BEFORE UPDATE OF input_history_snapshot_json ON recording_sessions
WHEN OLD.input_history_snapshot_json IS NOT NEW.input_history_snapshot_json
BEGIN SELECT RAISE(ABORT, 'Historical input snapshot is immutable'); END;
