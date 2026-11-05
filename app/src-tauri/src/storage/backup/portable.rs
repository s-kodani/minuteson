//! Trusted schema inventory, scope validation and native metadata redaction.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(super) const MAX_ROW_BYTES: usize = 32 * 1024 * 1024;
pub(super) const TABLES: &[&str] = &[
    "meetings",
    "file_assets",
    "recording_sessions",
    "audio_imports",
    "audio_tracks",
    "audio_chunks",
    "track_clock_segments",
    "audio_gaps",
    "track_inputs",
    "meeting_audio_commits",
    "analysis_revisions",
    "speakers",
    "speaker_segments",
    "utterances",
    "utterance_speakers",
    "analysis_input_snapshots",
    "minutes_markdown",
    "working_drafts",
    "processing_jobs",
    "job_checkpoints",
    "export_records",
    "vocabulary_scopes",
    "vocabulary_entries",
    "vocabulary_snapshots",
    "diarization_manual_assignments",
    "analysis_flows",
    "post_analysis_intents",
    "audio_import_commits",
    "audio_import_options",
    "restore_provenance",
    "voice_people",
    "voice_samples",
    "voice_sample_audio",
    "voice_matches",
    "voice_policies",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub row_count: u64,
}

pub(super) fn schema() -> Result<Connection, String> {
    initialize(Connection::open_in_memory().map_err(|e| e.to_string())?)
}

pub(super) fn initialize(mut db: Connection) -> Result<Connection, String> {
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA cache_size=-8192; PRAGMA temp_store=FILE;")
        .map_err(|e| e.to_string())?;
    initialize_database(&mut db).map_err(|e| e.to_string())?;
    Ok(db)
}

pub(super) fn tables(db: &Connection) -> Result<Vec<Table>, String> {
    let actual: BTreeSet<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    verify_database_schema(db).map_err(|e| e.to_string())?;
    let expected: BTreeSet<String> = TABLES
        .iter()
        .chain(
            [
                "app_settings",
                "schema_migrations",
                "restore_batches",
                "restore_identity_maps",
                "maintenance_operations",
                "maintenance_paths",
                "model_compatibility_checks",
            ]
            .iter(),
        )
        .chain([&"voice_audio_operations"])
        .chain(crate::storage::model_metadata::TABLES.iter())
        .map(|s| (*s).to_owned())
        .collect();
    if actual != expected {
        return Err("未対応の保存テーブルがあります。バックアップ形式を更新してください".into());
    }
    TABLES
        .iter()
        .map(|&name| {
            let columns = db
                .prepare(&format!("PRAGMA table_info(\"{name}\")"))
                .map_err(|e| e.to_string())?
                .query_map([], |r| {
                    Ok(Column {
                        name: r.get(1)?,
                        data_type: r.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            Ok(Table {
                name: name.into(),
                columns,
                row_count: 0,
            })
        })
        .collect()
}

pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn validate_scopes(db: &Connection) -> Result<(), String> {
    crate::storage::voice::validate_registry(db)?;
    crate::storage::post_analysis::validate(db)?;
    crate::storage::minutes_markdown::verify_all(db)?;
    let invalid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM meetings m JOIN analysis_revisions r ON r.id=m.current_analysis_id WHERE r.meeting_id!=m.id OR r.publication_state!='committed')",
        [], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if invalid {
        return Err("バックアップの現在版が別会議を参照しています".into());
    }
    for sql in [
        "SELECT EXISTS(SELECT 1 FROM analysis_flows f JOIN processing_jobs j ON j.id=f.transcription_job_id WHERE j.meeting_id!=f.meeting_id OR j.kind!='transcription')",
        "SELECT EXISTS(SELECT 1 FROM analysis_flows f JOIN processing_jobs j ON j.id=f.diarization_job_id WHERE j.meeting_id!=f.meeting_id OR j.kind!='diarization')",
        "SELECT EXISTS(SELECT 1 FROM audio_import_commits p JOIN file_assets f ON f.id=p.manifest_asset_id WHERE f.meeting_id!=p.meeting_id OR f.content_hash!=p.manifest_hash)",
        "SELECT EXISTS(SELECT 1 FROM vocabulary_scopes WHERE (meeting_id IS NULL AND id!='global') OR (meeting_id IS NOT NULL AND id!='meeting:'||meeting_id))",
    ] {
        if db.query_row(sql, [], |r| r.get::<_, bool>(0)).map_err(|e| e.to_string())? {
            return Err("バックアップの会議・入力の参照が不正です".into());
        }
    }
    Ok(())
}
