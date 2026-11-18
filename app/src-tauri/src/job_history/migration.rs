//! One-time device history v1 -> v2 conversion. Legacy identifiers and kinds
//! are deliberately confined here; the live service only accepts v2 entries.
use super::{read, Entry};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;

const INVALID: &str = "録音の保存履歴を統合できません。元の記録を保持しています。";

fn uuid(value: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| INVALID.to_string())?;
    if parsed.to_string() != value {
        return Err(INVALID.into());
    }
    Ok(())
}

fn session(entry: &Entry) -> Result<&str, String> {
    let value = if entry.kind == "recording_save" {
        entry.id.strip_prefix("recording_save:").ok_or(INVALID)?
    } else {
        let value = entry
            .id
            .strip_prefix("recording_recovery:")
            .ok_or(INVALID)?;
        let (run, session) = value.split_once(':').ok_or(INVALID)?;
        uuid(run)?;
        session
    };
    uuid(value)?;
    uuid(entry.meeting_id.as_deref().ok_or(INVALID)?)?;
    Ok(value)
}

fn confirmed(entry: &Entry) -> bool {
    ["succeeded", "failed", "cancelled"].contains(&entry.state.as_str())
}

pub(super) fn initialize(db: &Connection, version: i64) -> Result<(), String> {
    if version == 2 {
        return Ok(());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS history(id TEXT PRIMARY KEY,kind TEXT NOT NULL,target TEXT NOT NULL,meeting_id TEXT,state TEXT NOT NULL,accepted_at INTEGER,finished_at INTEGER,observed_at INTEGER NOT NULL,detail TEXT,active INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS history_order ON history(active,coalesce(finished_at,observed_at) DESC,id DESC);
        CREATE INDEX IF NOT EXISTS history_meeting ON history(meeting_id);
        CREATE TABLE IF NOT EXISTS removed_meetings(id TEXT PRIMARY KEY,removed_at INTEGER NOT NULL);")
        .map_err(|e| e.to_string())?;
    let rows = {
        let mut q = tx.prepare("SELECT id,kind,target,meeting_id,state,accepted_at,finished_at,observed_at,detail FROM history WHERE kind IN ('recording_save','recording_recovery') ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = q.query_map([], read).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    // The canonical ID includes the session UUID. A UUID associated with two
    // meetings is inconsistent rather than permission to merge those meetings.
    let mut merged: BTreeMap<String, (Entry, bool)> = BTreeMap::new();
    for mut entry in rows {
        let session = session(&entry)?.to_owned();
        let original_save = entry.kind == "recording_save";
        entry.id = format!("recording_save:{session}");
        entry.kind = "recording_save".into();
        entry.validate()?;
        match merged.get_mut(&session) {
            None => {
                merged.insert(session, (entry, original_save));
            }
            Some((old, old_save)) => {
                if old.meeting_id != entry.meeting_id {
                    return Err(INVALID.into());
                }
                let accepted = match (old.accepted_at, entry.accepted_at) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                let replace = (entry.observed_at, confirmed(&entry), original_save)
                    > (old.observed_at, confirmed(old), *old_save);
                if replace {
                    if entry.target.is_empty() {
                        entry.target = old.target.clone();
                    }
                    *old = entry;
                    *old_save = original_save;
                }
                old.accepted_at = accepted;
            }
        }
    }
    tx.execute(
        "DELETE FROM history WHERE kind IN ('recording_save','recording_recovery')",
        [],
    )
    .map_err(|e| e.to_string())?;
    for (entry, _) in merged.values() {
        tx.execute(
            "INSERT INTO history VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                entry.id,
                entry.kind,
                entry.target,
                entry.meeting_id,
                entry.state,
                entry.accepted_at,
                entry.finished_at,
                entry.observed_at,
                entry.detail,
                entry.active()
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.pragma_update(None, "user_version", 2)
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
