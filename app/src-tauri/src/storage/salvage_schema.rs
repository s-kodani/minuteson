//! Additive migration for rescue provenance. Keep the published 39/40
//! inventories and old deletion evidence stable.
use super::*;
use std::collections::BTreeMap;

const MEETINGS: &str = "CREATE TABLE salvage_meetings (
    meeting_id TEXT PRIMARY KEY NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL CHECK (json_valid(record_json) AND length(CAST(record_json AS BLOB))<=16384)
)";
const RECEIPT: &str = "CREATE TABLE salvage_receipt (
    singleton INTEGER PRIMARY KEY CHECK (singleton=1),
    result_json TEXT NOT NULL CHECK (json_valid(result_json) AND length(CAST(result_json AS BLOB))<=33554432)
)";

pub(super) fn owns_object(name: &str) -> bool {
    matches!(name, "salvage_meetings" | "salvage_receipt")
}

pub(super) fn verify(db: &Connection) -> Result<bool, Box<dyn Error>> {
    let mut query = db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name IN ('salvage_meetings','salvage_receipt') OR tbl_name IN ('salvage_meetings','salvage_receipt') ORDER BY name")?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let explicit = rows
        .iter()
        .filter(|(_, _, sql)| sql.is_some())
        .collect::<Vec<_>>();
    if explicit.is_empty() && rows.is_empty() {
        return Ok(false);
    }
    if explicit.len() != 2
        || rows.len() != 3
        || !rows.iter().any(|(kind, name, sql)| {
            kind == "index" && name == "sqlite_autoindex_salvage_meetings_1" && sql.is_none()
        })
        || explicit
            .iter()
            .zip([("salvage_meetings", MEETINGS), ("salvage_receipt", RECEIPT)])
            .any(|((kind, name, sql), (expected, definition))| {
                kind != "table" || name != expected || sql.as_deref() != Some(definition)
            })
    {
        return Err("データ復元の保存形式が既知の定義と一致しません".into());
    }
    Ok(true)
}

pub(super) fn install(db: &Connection) -> Result<(), rusqlite::Error> {
    db.execute_batch(&format!("{MEETINGS}; {RECEIPT};"))
}

pub(super) fn migrate(db: &mut Connection) -> Result<(), Box<dyn Error>> {
    if !verify(db)? {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !verify(&tx)? {
            install(&tx)?;
        }
        tx.commit()?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MeetingRecord {
    pub(super) operation_id: String,
    pub(super) source_meeting_id: String,
    pub(super) audio_unavailable: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(super) audio_deleted: bool,
    pub(super) notes: BTreeMap<String, String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl MeetingRecord {
    pub(super) fn audio_absent(&self) -> bool {
        self.audio_deleted || self.audio_unavailable.is_some()
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        let identifier = |id: &str| Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id);
        if !identifier(&self.operation_id)
            || !identifier(&self.source_meeting_id)
            || (self.audio_deleted && self.audio_unavailable.is_some())
            || self
                .audio_unavailable
                .as_ref()
                .is_some_and(|reason| reason.is_empty() || reason.len() > 2000)
            || self.notes.iter().any(|(key, reason)| {
                !matches!(
                    key.as_str(),
                    "meeting" | "transcript" | "draft" | "minutes" | "vocabulary"
                ) || reason.is_empty()
                    || reason.len() > 2000
            })
        {
            return Err("会議データの復元記録が不正です".into());
        }
        Ok(())
    }
}

pub(super) fn insert_meeting(
    db: &Connection,
    id: &str,
    record: &MeetingRecord,
) -> Result<(), String> {
    let mut current = record.clone();
    current.notes.remove("draft");
    current.validate()?;
    validate_deleted_audio(db, id, &current)?;
    let json = serde_json::to_string(&current).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO salvage_meetings(meeting_id,record_json) VALUES(?1,?2)",
        params![id, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn meeting(db: &Connection, id: &str) -> Result<Option<MeetingRecord>, String> {
    let present: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='salvage_meetings')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !present {
        return Ok(None);
    }
    db.query_row("SELECT CASE WHEN length(CAST(record_json AS BLOB))<=16384 THEN record_json END FROM salvage_meetings WHERE meeting_id=?1",
        [id], |r| r.get::<_, String>(0)).optional().map_err(|e| e.to_string())?
        .map(|json| {
            let mut record: MeetingRecord = serde_json::from_str(&json).map_err(|e|e.to_string())?;
            // A previous rescue report can mention an excluded draft. An
            // already recovered confirmed transcript has no draft to repair.
            record.notes.remove("draft");
            record.validate()?;
            validate_deleted_audio(db, id, &record)?;
            Ok(record)
        }).transpose()
}

fn validate_deleted_audio(
    db: &Connection,
    meeting: &str,
    record: &MeetingRecord,
) -> Result<(), String> {
    if !record.audio_deleted {
        return Ok(());
    }
    let available: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state!='missing')
             OR EXISTS(SELECT 1 FROM audio_chunks WHERE meeting_id=?1 AND state!='missing')
             OR EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND state!='failed')",
        [meeting], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if available {
        return Err("復元された音声削除の記録が現在の音声状態と一致しません".into());
    }
    Ok(())
}

pub(super) fn append_meetings(db: &Connection, source: &Connection) -> Result<(), String> {
    if !verify(source).map_err(|e| e.to_string())? {
        return Ok(());
    }
    let mut query = source
        .prepare("SELECT meeting_id,record_json FROM salvage_meetings ORDER BY meeting_id")
        .map_err(|e| e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        let json: String = row.get(1).map_err(|e| e.to_string())?;
        let record = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        insert_meeting(db, &id, &record)?;
    }
    Ok(())
}
