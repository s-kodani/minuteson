//! Typed fixed-data reconstruction. Results stay in private restore staging;
//! this module does not publish meetings or make missing engines available.
use super::restore_ids::RestoreIds;
use super::*;
use crate::audio_import::{ImportManifest, ImportOptions};
use crate::storage::draft_splits::UtteranceSplit;
use crate::storage::drafts::{ManualSpeaker, SpeakerMerge};
use crate::storage::jobs::{fixed_spec_hash, JobSpec};
use crate::storage::transcription::RecordedAudioPlan;
use crate::storage::vocabulary::VocabularySnapshot;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod checkpoints;
mod config;
mod order;
pub(in crate::storage::backup) mod workspace;

pub(super) struct Definition {
    pub domain: &'static str,
    pub meeting: String,
    pub old: String,
    pub lineage: Option<&'static str>,
}

pub(super) fn visit_definitions(
    source: &Connection,
    cancelled: &impl Fn() -> bool,
    visit: &mut impl FnMut(Definition) -> Result<(), String>,
) -> Result<(), String> {
    for sql in [
        "SELECT meeting_id,payload_json,'vocabulary' FROM vocabulary_snapshots",
        "SELECT meeting_id,parameters_json,'parameters' FROM processing_jobs",
        "SELECT meeting_id,config_json,'config' FROM analysis_revisions",
        "SELECT i.meeting_id,o.options_json,'options' FROM audio_import_options o JOIN audio_imports i ON i.id=o.import_id",
    ] {
        let mut query=source.prepare(sql).map_err(|e|e.to_string())?;
        let mut rows=query.query([]).map_err(|e|e.to_string())?;
        while let Some(row)=rows.next().map_err(|e|e.to_string())? {
            check_cancelled(cancelled)?;
            let meeting:String=row.get(0).map_err(|e|e.to_string())?;
            let text:String=row.get(1).map_err(|e|e.to_string())?;
            let kind:String=row.get(2).map_err(|e|e.to_string())?;
            let value:Value=serde_json::from_str(&text).map_err(|e|e.to_string())?;
            let vocabulary=if kind=="vocabulary" {Some(&value)} else {value.get("vocabulary").filter(|v|!v.is_null())};
            if let Some(value)=vocabulary {
                let snapshot:VocabularySnapshot=serde_json::from_value(value.clone()).map_err(|e|e.to_string())?;
                snapshot.validate()?;
                if snapshot.meeting_id!=meeting {return Err("固定用語の会議が一致しません".into());}
                visit(Definition{domain:"vocabulary_snapshots",meeting:meeting.clone(),old:snapshot.id,lineage:None})?;
                for entry in snapshot.entries {
                    visit(Definition{domain:"vocabulary_entries",meeting:if entry.scope=="global" {String::new()} else {meeting.clone()},old:entry.entry.id,lineage:None})?;
                }
            }
            if kind=="options" {
                let options:ImportOptions=serde_json::from_value(value.clone()).map_err(|e|e.to_string())?;
                options.validate()?;
                for entry in options.meeting_vocabulary {visit(Definition{domain:"vocabulary_entries",meeting:meeting.clone(),old:entry.id,lineage:None})?;}
            }
            if kind=="config" {
                if let Some(operations)=value.get("manualSpeakerOperations") {
                    let operations:ManualSpeakerOperations=serde_json::from_value(operations.clone()).map_err(|e|e.to_string())?;
                    if operations.schema_version!=1 {return Err("手動話者の由来版が不正です".into());}
                    for speaker in operations.added {visit(Definition{domain:"speakers",meeting:meeting.clone(),old:speaker.id,lineage:Some("speaker_lineage")})?;}
                }
                if let Some(splits)=value.get("manualUtteranceSplits") {
                    let operations:Vec<SplitOperation>=serde_json::from_value(splits.clone()).map_err(|e|e.to_string())?;
                    for part in operations.into_iter().flat_map(|o|o.splits).flat_map(|s|s.parts) {
                        visit(Definition{domain:"utterances",meeting:meeting.clone(),old:part.id,lineage:Some("utterance_lineage")})?;
                    }
                }
                if let Some(lineages)=value.get("sourceRecognitionLineages") {
                    let lineages:Vec<String>=serde_json::from_value(lineages.clone()).map_err(|e|e.to_string())?;
                    for old in lineages {visit(Definition{domain:"utterance_lineage",meeting:meeting.clone(),old,lineage:None})?;}
                }
            }
        }
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ManualSpeakerOperations {
    schema_version: u32,
    added: Vec<ManualSpeaker>,
    merges: Vec<SpeakerMerge>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SplitOperation {
    source_analysis_id: String,
    source_content_hash: String,
    splits: Vec<UtteranceSplit>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DraftOrigin {
    id: String,
    lock_version: i64,
    content_hash: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct InputSnapshot {
    pub schema_version: u32,
    pub source_analysis_id: String,
    pub source_content_hash: String,
    source_draft: Option<DraftOrigin>,
    pub working_content_hash: String,
    pub meeting_title: String,
    pub meeting_created_at: i64,
    pub meeting_started_at_utc_ms: Option<i64>,
}

pub(super) struct FixedData<'p, 's> {
    // Both connections close before this operation's registry directory.
    registry: Connection,
    ids: &'p RestoreIds<'s>,
    stage: scratch::Stage,
}

impl<'p, 's> FixedData<'p, 's> {
    /// Rebuild typed durable documents. Revision/job dependency hashes are
    /// calculated later, after their referenced rows have been reconstructed.
    /// This row alone is not a publishable restored database.
    pub(super) fn typed_row(
        &self,
        table: &portable::Table,
        rowid: i64,
    ) -> Result<Vec<rusqlite::types::Value>, String> {
        use rusqlite::types::Value as SqlValue;
        if matches!(
            table.name.as_str(),
            "working_drafts" | "edit_operations" | "transcript_edit_recovery"
        ) {
            return self.ids.draft_row(&table.name, rowid);
        }
        let mut row = self.ids.relational_row(&table.name, rowid)?;
        let column = |name: &str| {
            table
                .columns
                .iter()
                .position(|c| c.name == name)
                .ok_or_else(|| format!("復元列がありません: {name}"))
        };
        match table.name.as_str() {
            "recording_sessions" => {
                if let Some(index) = table
                    .columns
                    .iter()
                    .position(|c| c.name == "input_delivery_transfer_json")
                {
                    if let SqlValue::Text(text) = &row[index] {
                        let snapshot_index = column("input_history_snapshot_json")?;
                        // Relational IDs are already remapped, while immutable
                        // bound JSON below still belongs to the archive row.
                        let (old_meeting, old_session): (String, String) = self
                            .ids
                            .source()
                            .database
                            .query_row(
                                "SELECT meeting_id,id FROM recording_sessions WHERE rowid=?1",
                                params![rowid],
                                |r| Ok((r.get(0)?, r.get(1)?)),
                            )
                            .map_err(|e| e.to_string())?;
                        let source = super::input_snapshots::Bound::decode(
                            row_text(&row, snapshot_index)?,
                            &old_meeting,
                            &old_session,
                        )?
                        .into_snapshot();
                        let transfer = crate::storage::restored_input::Transfer::decode(
                            text,
                            &old_meeting,
                            &old_session,
                            &source,
                        )?;
                        row[index] = SqlValue::Text(transfer.rebind(
                            row_text(&row, column("meeting_id")?)?,
                            row_text(&row, column("id")?)?,
                            &source,
                        )?);
                    }
                }
                if let Some(index) = table
                    .columns
                    .iter()
                    .position(|c| c.name == "input_history_snapshot_json")
                {
                    if let SqlValue::Text(text) = &row[index] {
                        let (old_meeting, old_session): (String, String) = self
                            .ids
                            .source()
                            .database
                            .query_row(
                                "SELECT meeting_id,id FROM recording_sessions WHERE rowid=?1",
                                params![rowid],
                                |r| Ok((r.get(0)?, r.get(1)?)),
                            )
                            .map_err(|e| e.to_string())?;
                        let bound = super::input_snapshots::Bound::decode(
                            text,
                            &old_meeting,
                            &old_session,
                        )?;
                        row[index] = SqlValue::Text(bound.rebind(
                            row_text(&row, column("meeting_id")?)?,
                            row_text(&row, column("id")?)?,
                        )?);
                    }
                }
            }
            "minutes_revisions" => return Err("旧形式の構造化議事録には対応していません".into()),
            "audio_import_options" => {
                let meeting:String=self.ids.source().database.query_row("SELECT i.meeting_id FROM audio_import_options o JOIN audio_imports i ON i.id=o.import_id WHERE o.rowid=?1",params![rowid],|r|r.get(0)).map_err(|e|e.to_string())?;
                let value = self.import_options(
                    &meeting,
                    &serde_json::from_str(row_text(&row, column("options_json")?)?)
                        .map_err(|e| e.to_string())?,
                )?;
                let encoded = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                row[column("options_hash")?] =
                    SqlValue::Text(format!("{:x}", Sha256::digest(encoded.as_bytes())));
                row[column("options_json")?] = SqlValue::Text(encoded);
            }
            "vocabulary_snapshots" => {
                let meeting: String = self
                    .ids
                    .source()
                    .database
                    .query_row(
                        "SELECT meeting_id FROM vocabulary_snapshots WHERE rowid=?1",
                        params![rowid],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let value = self.vocabulary(
                    &meeting,
                    &serde_json::from_str(row_text(&row, column("payload_json")?)?)
                        .map_err(|e| e.to_string())?,
                )?;
                row[column("content_hash")?] = SqlValue::Text(
                    value["contentHash"]
                        .as_str()
                        .ok_or("固定用語ハッシュがありません")?
                        .into(),
                );
                row[column("payload_json")?] =
                    SqlValue::Text(serde_json::to_string(&value).map_err(|e| e.to_string())?);
            }
            _ => {}
        }
        Ok(row)
    }

    pub(super) fn new(
        ids: &'p RestoreIds<'s>,
        parent: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let rows = ids
            .source()
            .manifest
            .tables
            .iter()
            .try_fold(0_u64, |total, table| {
                total
                    .checked_add(table.row_count)
                    .ok_or("固定ハッシュ行数が範囲外です")
            })?;
        check_capacity(
            parent,
            rows.checked_mul(1024)
                .ok_or("固定ハッシュ領域の見積もりが範囲外です")?,
        )?;
        let stage = archive::private_stage(parent)?;
        let path = stage.path().join("fixed-hashes.sqlite3");
        private_file(&path)?;
        let registry = Connection::open(path).map_err(|e| e.to_string())?;
        registry.execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=FULL; PRAGMA cache_size=-8192;
            CREATE TABLE hashes(domain TEXT NOT NULL,meeting TEXT NOT NULL,old_id TEXT NOT NULL,old_hash TEXT NOT NULL,new_hash TEXT NOT NULL,
                PRIMARY KEY(domain,meeting,old_id,old_hash));
            CREATE TABLE assets(meeting TEXT NOT NULL,old_id TEXT NOT NULL,old_hash TEXT NOT NULL,new_hash TEXT NOT NULL,new_size INTEGER NOT NULL,
                PRIMARY KEY(meeting,old_id));
            BEGIN IMMEDIATE;").map_err(|e|e.to_string())?;
        let result = Self {
            registry,
            ids,
            stage,
        };
        let source = &ids.source().database;
        for sql in [
            "SELECT id,meeting_id,target_kind,payload_json FROM working_drafts",
            "SELECT d.id,d.meeting_id,d.target_kind,e.payload_json FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id",
            "SELECT d.id,d.meeting_id,d.target_kind,e.inverse_json FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id",
        ] {
            let mut query=source.prepare(sql).map_err(|e|e.to_string())?;
            let mut rows=query.query([]).map_err(|e|e.to_string())?;
            while let Some(row)=rows.next().map_err(|e|e.to_string())? {
                check_cancelled(cancelled)?;
                let id:String=row.get(0).map_err(|e|e.to_string())?;
                let meeting:String=row.get(1).map_err(|e|e.to_string())?;
                let kind:String=row.get(2).map_err(|e|e.to_string())?;
                let text:String=row.get(3).map_err(|e|e.to_string())?;
                let old_hash=match kind.as_str() {"transcript"=>super::super::drafts::draft_hash(&text),"minutes"=>return Err("旧形式の議事録下書きには対応していません".into()),_=>return Err("復元下書きの種別が不正です".into())};
                let (_,new_hash)=ids.draft_json(&kind,&meeting,&text)?;
                result.remember("working_drafts",&meeting,&id,&old_hash,&new_hash)?;
            }
        }
        result.prepare_order(cancelled)?;
        result
            .registry
            .execute_batch("COMMIT;")
            .map_err(|e| e.to_string())?;
        Ok(result)
    }

    fn remember(
        &self,
        domain: &str,
        meeting: &str,
        id: &str,
        old: &str,
        new: &str,
    ) -> Result<(), String> {
        if !portable::valid_hash(old) || !portable::valid_hash(new) {
            return Err("復元内容ハッシュが不正です".into());
        }
        if let Some(existing)=self.registry.query_row("SELECT new_hash FROM hashes WHERE domain=?1 AND meeting=?2 AND old_id=?3 AND old_hash=?4",params![domain,meeting,id,old],|r|r.get::<_,String>(0)).optional().map_err(|e|e.to_string())? {
            if existing!=new {return Err("同じ復元由来が異なるハッシュへ対応しています".into());}
            return Ok(());
        }
        self.registry
            .execute(
                "INSERT INTO hashes VALUES(?1,?2,?3,?4,?5)",
                params![domain, meeting, id, old, new],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn resolve(
        &self,
        domain: &str,
        meeting: &str,
        id: &str,
        old: &str,
    ) -> Result<String, String> {
        self.ids.get(domain, meeting, id)?;
        self.registry.query_row("SELECT new_hash FROM hashes WHERE domain=?1 AND meeting=?2 AND old_id=?3 AND old_hash=?4",params![domain,meeting,id,old],|r|r.get(0)).optional().map_err(|e|e.to_string())?
            .ok_or_else(||format!("参照元ハッシュが不正か再構成前です: {domain}"))
    }

    pub(super) fn vocabulary_book_hash(
        &self,
        target: &Connection,
        old_scope: &str,
    ) -> Result<String, String> {
        let mapped = if old_scope == "global" {
            "global".to_owned()
        } else {
            let meeting = old_scope
                .strip_prefix("meeting:")
                .ok_or("用語リストの所有情報が不正です")?;
            format!("meeting:{}", self.ids.get("meetings", "", meeting)?)
        };
        super::super::vocabulary::stored_book_hash(target, &mapped)
    }

    pub(super) fn vocabulary(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        let mut snapshot: VocabularySnapshot =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        snapshot.validate()?;
        if snapshot.meeting_id != meeting {
            return Err("固定用語の会議が一致しません".into());
        }
        let saved:String=self.ids.source().database.query_row("SELECT payload_json FROM vocabulary_snapshots WHERE id=?1 AND meeting_id=?2 AND content_hash=?3",params![snapshot.id,meeting,snapshot.content_hash],|r|r.get(0)).map_err(|e|e.to_string())?;
        if serde_json::from_str::<Value>(&saved).map_err(|e| e.to_string())? != *value {
            return Err("固定用語の由来が保存情報と一致しません".into());
        }
        let old_id = snapshot.id.clone();
        let old_hash = snapshot.content_hash.clone();
        snapshot.id = self.ids.get("vocabulary_snapshots", meeting, &old_id)?;
        snapshot.meeting_id = self.ids.get("meetings", "", meeting)?;
        for entry in &mut snapshot.entries {
            entry.entry.id = self.ids.get(
                "vocabulary_entries",
                if entry.scope == "global" { "" } else { meeting },
                &entry.entry.id,
            )?;
        }
        snapshot.content_hash = snapshot.digest()?;
        snapshot.validate()?;
        self.remember(
            "vocabulary_snapshots",
            meeting,
            &old_id,
            &old_hash,
            &snapshot.content_hash,
        )?;
        serde_json::to_value(snapshot).map_err(|e| e.to_string())
    }

    pub(super) fn import_options(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        let mut options: ImportOptions =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        options.validate()?;
        for entry in &mut options.meeting_vocabulary {
            entry.id = self.ids.get("vocabulary_entries", meeting, &entry.id)?;
        }
        options.validate()?;
        serde_json::to_value(options).map_err(|e| e.to_string())
    }

    pub(super) fn import_manifest(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        let mut manifest: ImportManifest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        manifest.info.validate()?;
        if manifest.schema_version != 1 || manifest.meeting_id != meeting {
            return Err("復元取り込みmanifestの会議・形式が不正です".into());
        }
        let (file_name,info,source_size,source_hash,audio_id):(String,String,i64,String,String)=self.ids.source().database.query_row("SELECT i.original_file_name,p.info_json,p.source_size,p.source_hash,i.converted_asset_id FROM audio_imports i JOIN audio_import_commits p ON p.import_id=i.id AND p.meeting_id=i.meeting_id WHERE i.id=?1 AND i.meeting_id=?2 AND i.state='ready'",params![manifest.import_id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
        let source = self.ids.source();
        let (audio_size,audio_hash,audio_state,media):(i64,String,String,String)=source.database.query_row("SELECT size_bytes,content_hash,state,media_type FROM file_assets WHERE id=?1 AND meeting_id=?2",params![audio_id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
        if source.deleted_audio.contains(meeting) {
            source
                .deleted_audio
                .verify_meeting(&source.database, meeting)?;
            let missing = source
                .deleted_audio
                .asset(meeting, &audio_id)
                .ok_or("取り込み変換音声の検査済み欠落履歴がありません")?;
            if audio_state != "missing"
                || audio_size != missing.size
                || audio_hash != missing.hash
                || media != missing.media
            {
                return Err("取り込み変換音声と検査済み欠落履歴が一致しません".into());
            }
        } else if audio_state != "ready"
            && !super::super::meeting_audio::superseded_asset(&source.database, &audio_id)?
        {
            return Err("取り込み変換音声が利用可能ではありません".into());
        }
        if !deleted_audio::audio_media(&media) || audio_size < 0 || source_size < 0 {
            return Err("取り込み音声の容量・種別が不正です".into());
        }
        if manifest.original_file_name != file_name
            || manifest.info
                != serde_json::from_str::<crate::audio_import::AudioInfo>(&info)
                    .map_err(|e| e.to_string())?
            || manifest.original.size != source_size as u64
            || manifest.original.sha256 != source_hash
            || manifest.audio.size != audio_size as u64
            || manifest.audio.sha256 != audio_hash
        {
            return Err("復元取り込みmanifestの音声由来が一致しません".into());
        }
        manifest.meeting_id = self.ids.get("meetings", "", meeting)?;
        manifest.import_id = self
            .ids
            .get("audio_imports", meeting, &manifest.import_id)?;
        serde_json::to_value(manifest).map_err(|e| e.to_string())
    }

    pub(super) fn asset_bytes(
        &self,
        meeting: &str,
        id: &str,
        bytes: &[u8],
    ) -> Result<(u64, String), String> {
        if bytes.len() > portable::MAX_ROW_BYTES {
            return Err("管理JSONが容量上限を超えています。音声はファイルで照合します".into());
        }
        self.register_asset(
            meeting,
            id,
            bytes.len() as u64,
            format!("{:x}", Sha256::digest(bytes)),
        )
    }

    pub(super) fn asset_file(
        &self,
        meeting: &str,
        id: &str,
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(u64, String), String> {
        if self.ids.source().deleted_audio.asset(meeting, id).is_some() {
            return Err("削除済み音声を実ファイルとして登録しません".into());
        }
        let (size, hash) = digest_file(path, cancelled)?;
        self.register_asset(meeting, id, size, hash)
    }

    // Register only immutable absence metadata. No path/body or ready state is
    // produced, and ordinary body registration cannot adopt this missing input.
    pub(super) fn absent_source_asset(&self, meeting: &str, id: &str) -> Result<(), String> {
        let source = self.ids.source();
        let missing = source
            .deleted_audio
            .asset(meeting, id)
            .ok_or("音声資産の検査済み欠落履歴がありません")?;
        let current: (String,i64,String,String) = source.database.query_row(
            "SELECT media_type,size_bytes,content_hash,state FROM file_assets WHERE id=?1 AND meeting_id=?2",
            params![id,meeting], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).map_err(|e|e.to_string())?;
        if current
            != (
                missing.media.clone(),
                missing.size,
                missing.hash.clone(),
                "missing".into(),
            )
        {
            return Err("音声資産と検査済み欠落履歴が一致しません".into());
        }
        self.ids.get("file_assets", meeting, id)?;
        self.remember_asset(
            meeting,
            id,
            &missing.hash,
            missing.size,
            missing.hash.clone(),
        )?;
        Ok(())
    }

    pub(super) fn absent_superseded_asset(&self, meeting: &str, id: &str) -> Result<(), String> {
        let source = self.ids.source();
        if !super::super::meeting_audio::superseded_asset(&source.database, id)? {
            return Err("置換済み音声の検査済み履歴がありません".into());
        }
        self.ids.get("file_assets", meeting, id)?;
        let (size, hash): (i64, String) = source
            .database
            .query_row(
                "SELECT size_bytes,content_hash FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![id, meeting],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        self.remember_asset(meeting, id, &hash, size, hash.clone())?;
        Ok(())
    }

    fn register_asset(
        &self,
        meeting: &str,
        id: &str,
        size: u64,
        new: String,
    ) -> Result<(u64, String), String> {
        if self.ids.source().deleted_audio.asset(meeting, id).is_some() {
            return Err("削除済み音声を実ファイルとして登録しません".into());
        }
        let (old,old_size,media): (String,i64,String) = self
            .ids
            .source()
            .database
            .query_row(
                "SELECT content_hash,size_bytes,media_type FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![id, meeting],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        self.ids.get("file_assets", meeting, id)?;
        let size = i64::try_from(size).map_err(|_| "復元ファイル容量が範囲外です")?;
        if deleted_audio::audio_media(&media) && (new != old || size != old_size) {
            return Err("復元で音声本体が変更されています".into());
        }
        self.remember_asset(meeting, id, &old, size, new)
    }

    fn remember_asset(
        &self,
        meeting: &str,
        id: &str,
        old: &str,
        size: i64,
        new: String,
    ) -> Result<(u64, String), String> {
        self.remember("file_assets", meeting, id, old, &new)?;
        self.registry
            .execute(
                "INSERT INTO assets VALUES(?1,?2,?3,?4,?5) ON CONFLICT(meeting,old_id) DO NOTHING",
                params![meeting, id, old, new, size],
            )
            .map_err(|e| e.to_string())?;
        Ok((size as u64, new))
    }

    pub(super) fn asset_hash(
        &self,
        meeting: &str,
        id: &str,
        old: &str,
    ) -> Result<(i64, String), String> {
        self.ids.get("file_assets", meeting, id)?;
        self.registry.query_row("SELECT new_size,new_hash FROM assets WHERE meeting=?1 AND old_id=?2 AND old_hash=?3",params![meeting,id,old],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?.ok_or("管理ファイルの由来が不正か再構成前です".into())
    }

    pub(super) fn audio_plan(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        self.analysis_audio_plan(meeting, value, None)
    }

    pub(super) fn analysis_audio_plan(
        &self,
        meeting: &str,
        value: &Value,
        known: Option<&super::super::transcription::known::KnownInput>,
    ) -> Result<Value, String> {
        let mut plan: RecordedAudioPlan =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        let source = self.ids.source();
        let mut expected = if known.is_some() {
            super::super::transcription::read_analysis_audio_plan(
                &source.database,
                meeting,
                source
                    .deleted_audio
                    .contains(meeting)
                    .then_some(&source.deleted_audio),
            )?
        } else if source.deleted_audio.contains(meeting) {
            super::super::transcription::read_historical_audio_plan(
                &source.database,
                meeting,
                &source.deleted_audio,
            )?
        } else {
            super::super::transcription::read_audio_plan(&source.database, meeting)?
        };
        if plan != expected && super::retired_audio::verifies(&source.database, meeting, &plan)? {
            expected = plan.clone();
        }
        if plan != expected {
            return Err("固定音声のトラック・区間・時刻・由来が元版と一致しません".into());
        }
        if let Some(known) = known {
            if *known
                != super::super::transcription::known::read(&source.database, meeting, &expected)?
            {
                return Err("固定した保存区間・欠損が復元元の記録と一致しません".into());
            }
            known.validate(&expected)?;
        }
        plan.manifest_hash = self.resolve(
            "file_assets",
            meeting,
            &plan.manifest_asset_id,
            &plan.manifest_hash,
        )?;
        plan.manifest_asset_id = self
            .ids
            .get("file_assets", meeting, &plan.manifest_asset_id)?;
        for track in &mut plan.tracks {
            track.id = self.ids.get("audio_tracks", meeting, &track.id)?;
            for chunk in &mut track.chunks {
                if super::super::meeting_audio::superseded_asset(&source.database, &chunk.asset_id)?
                {
                    self.absent_superseded_asset(meeting, &chunk.asset_id)?;
                }
                let (size, hash) = self.asset_hash(meeting, &chunk.asset_id, &chunk.hash)?;
                // Audio samples are not transformed. A changed waveform is never
                // accepted as an ID-only rebase of a fixed audio input.
                if size != chunk.size || hash != chunk.hash {
                    return Err("復元で音声本体が変更されています".into());
                }
                chunk.asset_id = self.ids.get("file_assets", meeting, &chunk.asset_id)?;
            }
        }
        serde_json::to_value(plan).map_err(|e| e.to_string())
    }

    pub(super) fn request(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        let mut request: MinutesJobRequest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if request.meeting_id != meeting {
            return Err("固定処理要求の会議が一致しません".into());
        }
        request.expected_content_hash = self.resolve(
            "analysis_revisions",
            meeting,
            &request.expected_analysis_id,
            &request.expected_content_hash,
        )?;
        request.expected_analysis_id =
            self.ids
                .get("analysis_revisions", meeting, &request.expected_analysis_id)?;
        request.operation_id = self.ids.get("operation", meeting, &request.operation_id)?;
        request.meeting_id = self.ids.get("meetings", "", meeting)?;
        if let Some(draft) = &mut request.expected_draft {
            draft.id = self.ids.get("working_drafts", meeting, &draft.id)?;
        }
        serde_json::to_value(request).map_err(|e| e.to_string())
    }

    pub(super) fn remember_revision(
        &self,
        target: &Transaction<'_>,
        domain: &str,
        meeting: &str,
        id: &str,
    ) -> Result<String, String> {
        if domain != "analysis_revisions" {
            return Err("復元版の種別が不正です".into());
        }
        let old: String = self
            .ids
            .source()
            .database
            .query_row(
                &format!("SELECT content_hash FROM {domain} WHERE id=?1 AND meeting_id=?2"),
                params![id, meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let new_id = self.ids.get(domain, meeting, id)?;
        let owned: bool = target
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {domain} WHERE id=?1 AND meeting_id=?2)"),
                params![new_id, self.ids.get("meetings", "", meeting)?],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !owned {
            return Err("再構成版の会議所属が一致しません".into());
        }
        let new = super::super::revisions::analysis_content_hash(target, &new_id)?;
        self.remember(domain, meeting, id, &old, &new)?;
        Ok(new)
    }

    pub(super) fn job_hash(
        &self,
        meeting: &str,
        id: &str,
        spec: &JobSpec,
        retry: Option<&str>,
        attempt: i64,
    ) -> Result<String, String> {
        if &self.job_spec(meeting, id)? != spec {
            return Err("再構成したジョブの固定条件が一致しません".into());
        }
        let (old,old_operation,old_retry,old_attempt):(String,String,Option<String>,i64)=self.ids.source().database.query_row("SELECT spec_hash,operation_id,retry_of_job_id,attempt FROM processing_jobs WHERE id=?1 AND meeting_id=?2",params![id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
        let expected_retry = old_retry
            .as_deref()
            .map(|r| self.ids.get("processing_jobs", meeting, r))
            .transpose()?;
        if spec.meeting_id != self.ids.get("meetings", "", meeting)?
            || spec.operation_id != self.ids.get("operation", meeting, &old_operation)?
            || expected_retry.as_deref() != retry
            || attempt != old_attempt
        {
            return Err("固定ジョブの会議・再試行の由来が一致しません".into());
        }
        let new = fixed_spec_hash(spec, retry, attempt)?;
        self.remember("processing_jobs", meeting, id, &old, &new)?;
        Ok(new)
    }
}

fn row_text(row: &[rusqlite::types::Value], index: usize) -> Result<&str, String> {
    match row.get(index) {
        Some(rusqlite::types::Value::Text(value)) => Ok(value),
        _ => Err("復元JSON列が不正です".into()),
    }
}

#[cfg(test)]
#[path = "restore_fixed_tests.rs"]
mod tests;
