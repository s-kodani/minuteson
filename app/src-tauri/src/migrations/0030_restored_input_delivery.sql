-- Immutable transfer proof is independent from the original producer snapshot.
ALTER TABLE recording_sessions ADD COLUMN input_delivery_transfer_json TEXT
    CHECK (input_delivery_transfer_json IS NULL OR
        (json_valid(input_delivery_transfer_json) AND
         length(CAST(input_delivery_transfer_json AS BLOB)) <= 263168));

CREATE TRIGGER recording_input_transfer_owner BEFORE INSERT ON recording_sessions
WHEN NEW.input_delivery_transfer_json IS NOT NULL AND
    (NEW.state NOT IN ('saved','interrupted','failed') OR
     NEW.input_history_snapshot_json IS NULL OR
     json_extract(NEW.input_delivery_transfer_json,'$.schemaVersion') IS NOT 1 OR
     json_extract(NEW.input_delivery_transfer_json,'$.meetingId') IS NOT NEW.meeting_id OR
     json_extract(NEW.input_delivery_transfer_json,'$.sessionId') IS NOT NEW.id)
BEGIN SELECT RAISE(ABORT, 'Delivery transfer requires its closed snapshot owner'); END;

CREATE TRIGGER recording_input_transfer_binding BEFORE UPDATE OF id,meeting_id,state ON recording_sessions
WHEN NEW.input_delivery_transfer_json IS NOT NULL AND
    (NEW.state NOT IN ('saved','interrupted','failed') OR
     json_extract(NEW.input_delivery_transfer_json,'$.meetingId') IS NOT NEW.meeting_id OR
     json_extract(NEW.input_delivery_transfer_json,'$.sessionId') IS NOT NEW.id)
BEGIN SELECT RAISE(ABORT, 'Delivery transfer owner cannot be changed'); END;

CREATE TRIGGER recording_input_transfer_fixed BEFORE UPDATE OF input_delivery_transfer_json ON recording_sessions
WHEN OLD.input_delivery_transfer_json IS NOT NEW.input_delivery_transfer_json
BEGIN SELECT RAISE(ABORT, 'Delivery transfer proof is immutable'); END;

-- Process-local writer incarnations are not portable. Backup synthesizes only
-- the bounded transfer proof from the committed state, without stamping source.
CREATE TABLE recording_input_delivery (
    session_id TEXT PRIMARY KEY REFERENCES recording_sessions(id) ON DELETE CASCADE,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    writer_id TEXT NOT NULL CHECK (length(writer_id)=36),
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    state_json TEXT NOT NULL CHECK (
        json_valid(state_json) AND length(CAST(state_json AS BLOB)) <= 262144 AND
        json_extract(state_json,'$.schemaVersion') IS 1 AND
        json_extract(state_json,'$.ownerId') IS session_id AND
        json_extract(state_json,'$.revision') IS revision)
);

CREATE TRIGGER recording_input_delivery_owner BEFORE INSERT ON recording_input_delivery
WHEN NOT EXISTS (SELECT 1 FROM recording_sessions WHERE id=NEW.session_id AND meeting_id=NEW.meeting_id
    AND state IN ('saved','interrupted','failed') AND input_history_snapshot_json IS NOT NULL)
BEGIN SELECT RAISE(ABORT, 'Delivery ledger requires a restored closed recording'); END;

CREATE TRIGGER recording_input_delivery_fixed BEFORE UPDATE OF session_id,meeting_id ON recording_input_delivery
WHEN OLD.session_id IS NOT NEW.session_id OR OLD.meeting_id IS NOT NEW.meeting_id
BEGIN SELECT RAISE(ABORT, 'Delivery ledger owner cannot be changed'); END;
