//! Private restore identity planning. This is NOT a publishable app database:
//! fixed JSON, file manifests and derived hashes must still be reconstructed.
use super::archive::VerifiedArchive;
use super::portable::{self, Table};
use super::*;
use crate::minutes_document::{Block, Inline, MinutesContent};
use rusqlite::types::Value;
use std::collections::BTreeMap;

const MAX_DRAFT_BYTES: usize = 2 * 1024 * 1024;

mod replay;

enum PreparationScope {
    Standard,
    HistoricalConstruction,
}

pub(crate) enum BatchPreparation<'a> {
    Pending(RestoreIds<'a>),
    AlreadyPublished {
        batch_id: String,
        archive_id: String,
    },
}

pub(crate) struct RestoreIds<'a> {
    // Close the disk-backed map before removing this operation's private files.
    database: Connection,
    source: &'a VerifiedArchive,
    tables: Vec<Table>,
    bindings: BTreeMap<String, BTreeMap<String, Rule>>,
    pub batch_id: String,
    pub archive_id: String,
    saved: Option<super::ledger::SavedBatch>,
    stage: scratch::Stage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    Entity(&'static str),
    SpeakerLineage,
    UtteranceLineage,
    Overlap,
    Operation,
    VocabularyScope,
    ProvenanceEntity,
}

fn entity_table(table: &str, column: &str) -> Result<&'static str, String> {
    if !portable::TABLES.contains(&table) {
        return Err("未知の復元ID領域です".into());
    }
    match (table, column) {
        (_, "meeting_id") => Ok("meetings"),
        (_, "analysis_revision_id") => Ok("analysis_revisions"),
        (_, "minutes_revision_id") => Ok("minutes_revisions"),
        (_, "track_id") => Ok("audio_tracks"),
        ("analysis_input_snapshots", "analysis_id") => Ok("analysis_revisions"),
        ("audio_import_commits" | "audio_import_options", "import_id") => Ok("audio_imports"),
        (table, "id") => portable::TABLES
            .iter()
            .copied()
            .find(|t| *t == table)
            .ok_or_else(|| "未知の復元ID領域です".into()),
        _ => Err("未対応の復元参照列です".into()),
    }
}

fn rules(db: &Connection, table: &Table) -> Result<BTreeMap<String, Rule>, String> {
    if !portable::TABLES.contains(&table.name.as_str()) {
        return Err("未知の復元テーブルです".into());
    }
    let mut result = BTreeMap::new();
    for c in &table.columns {
        let rule = match (table.name.as_str(), c.name.as_str()) {
            ("vocabulary_scopes", "id") => Some(Rule::VocabularyScope),
            ("restore_provenance", "entity_key") => Some(Rule::ProvenanceEntity),
            (_, "id") => Some(Rule::Entity(entity_table(&table.name, "id")?)),
            ("speakers", "lineage_id") => Some(Rule::SpeakerLineage),
            ("utterances", "lineage_id") => Some(Rule::UtteranceLineage),
            ("speaker_segments", "overlap_group_id") => Some(Rule::Overlap),
            (_, "operation_id" | "start_operation_id" | "stop_operation_id") => {
                Some(Rule::Operation)
            }
            ("export_records", "source_draft_id") => Some(Rule::Entity("working_drafts")),
            _ => None,
        };
        if let Some(rule) = rule {
            result.insert(c.name.clone(), rule);
        }
    }
    // Only schema installed by the application is used; never archive SQL.
    let mut statement = db
        .prepare(&format!("PRAGMA foreign_key_list(\"{}\")", table.name))
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let target: String = row.get(2).map_err(|e| e.to_string())?;
        let column: String = row.get(3).map_err(|e| e.to_string())?;
        let to: String = row.get(4).map_err(|e| e.to_string())?;
        let rule = if target == "vocabulary_scopes" && to == "id" {
            Rule::VocabularyScope
        } else {
            Rule::Entity(entity_table(&target, &to)?)
        };
        if result.insert(column, rule).is_some_and(|old| old != rule) {
            return Err("復元参照のID領域が一致しません".into());
        }
    }
    for column in &table.columns {
        if (column.name == "id" || column.name.ends_with("_id"))
            && !result.contains_key(&column.name)
        {
            return Err(format!(
                "未対応の復元ID列です: {}.{}",
                table.name, column.name
            ));
        }
    }
    Ok(result)
}

pub(super) fn owners_sql(table: &str) -> Result<String, String> {
    let sql = match table {
        "meetings" => "SELECT rowid,id FROM meetings".into(),
        "speakers" | "utterance_speakers" => format!("SELECT c.rowid,a.meeting_id FROM {table} c JOIN analysis_revisions a ON a.id=c.analysis_revision_id"),
        "minutes_items" | "evidence_links" => format!("SELECT c.rowid,m.meeting_id FROM {table} c JOIN minutes_revisions m ON m.id=c.minutes_revision_id"),
        "track_clock_segments" | "audio_gaps" => format!("SELECT c.rowid,t.meeting_id FROM {table} c JOIN audio_tracks t ON t.id=c.track_id"),
        "edit_operations" => "SELECT c.rowid,d.meeting_id FROM edit_operations c JOIN working_drafts d ON d.id=c.draft_id".into(),
        "job_checkpoints" => "SELECT c.rowid,j.meeting_id FROM job_checkpoints c JOIN processing_jobs j ON j.id=c.job_id".into(),
        "diarization_manual_assignments" => "SELECT c.rowid,a.meeting_id FROM diarization_manual_assignments c JOIN analysis_revisions a ON a.id=c.analysis_id".into(),
        "audio_import_options" => "SELECT c.rowid,i.meeting_id FROM audio_import_options c JOIN audio_imports i ON i.id=c.import_id".into(),
        "vocabulary_entries" => "SELECT c.rowid,s.meeting_id FROM vocabulary_entries c JOIN vocabulary_scopes s ON s.id=c.scope_id".into(),
        table if portable::TABLES.contains(&table) => format!("SELECT rowid,meeting_id FROM {table}"),
        _ => return Err("未知の復元テーブルです".into()),
    };
    Ok(sql)
}

fn scope<'a>(domain: &str, meeting: &'a str) -> &'a str {
    if domain == "meetings" {
        ""
    } else {
        meeting
    }
}

impl<'a> RestoreIds<'a> {
    pub(crate) fn prepare(
        archive: &'a VerifiedArchive,
        parent: &Path,
        batch: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::prepare_inner(
            archive,
            parent,
            batch,
            None,
            PreparationScope::Standard,
            cancelled,
        )
    }

    // Private construction only. Publication independently refuses deleted
    // source histories until the native unavailable-record pass is connected.
    pub(in crate::storage::backup) fn prepare_historical(
        archive: &'a VerifiedArchive,
        parent: &Path,
        batch: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if archive.deleted_audio.is_empty() {
            return Err("削除済み音声の履歴構築には検査済み履歴が必要です".into());
        }
        archive.verify_validation_database(cancelled)?;
        archive
            .deleted_audio
            .verify_omissions(&archive.manifest.omitted_assets)?;
        Self::prepare_inner(
            archive,
            parent,
            batch,
            None,
            PreparationScope::HistoricalConstruction,
            cancelled,
        )
    }

    pub(in crate::storage::backup) fn prepare_historical_for_batch(
        archive: &'a VerifiedArchive,
        parent: &Path,
        batch: &str,
        destination: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if archive.deleted_audio.is_empty() {
            return Err("削除済み音声の履歴再構築には検査済み履歴が必要です".into());
        }
        archive.verify_validation_database(cancelled)?;
        archive
            .deleted_audio
            .verify_omissions(&archive.manifest.omitted_assets)?;
        let ids = Self::prepare_inner(
            archive,
            parent,
            batch,
            Some(destination),
            PreparationScope::HistoricalConstruction,
            cancelled,
        )?;
        ids.verify_saved_destination(destination, cancelled)?;
        if super::ledger::saved_batch(destination, batch, archive, cancelled)? != ids.saved {
            return Err("履歴再構築中に保存済み復元バッチが変更されています".into());
        }
        Ok(ids)
    }

    fn prepare_inner(
        archive: &'a VerifiedArchive,
        parent: &Path,
        batch: &str,
        destination: Option<&Connection>,
        preparation_scope: PreparationScope,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if !Uuid::parse_str(batch).is_ok_and(|id| id.to_string() == batch) {
            return Err("復元バッチIDが不正です".into());
        }
        check_cancelled(cancelled)?;
        // Portable format 3 is validated and can retain text/history today;
        // native unavailable-history publication is the next construction pass.
        // Never fall through to ready-source reconstruction for such an archive.
        if matches!(preparation_scope, PreparationScope::Standard)
            && !archive.deleted_audio.is_empty()
        {
            return Err("削除済み音声を含むバックアップの追加復元はまだ未接続です。本文・根拠・音声削除履歴はアーカイブに保存されています".into());
        }
        let logical_bytes = archive
            .manifest
            .entries
            .first()
            .ok_or("復元論理データがありません")?
            .size;
        let rows = archive.manifest.tables.iter().try_fold(0_u64, |sum, t| {
            sum.checked_add(t.row_count).ok_or("復元行数が範囲外です")
        })?;
        let estimated = logical_bytes
            .checked_mul(8)
            .and_then(|bytes| {
                rows.checked_mul(1024)
                    .and_then(|overhead| bytes.checked_add(overhead))
            })
            .ok_or("復元ID領域の見積もりが範囲外です")?;
        check_capacity(parent, estimated)?;
        let saved = destination
            .map(|db| super::ledger::saved_batch(db, batch, archive, cancelled))
            .transpose()?
            .flatten();
        if saved
            .as_ref()
            .is_some_and(|s| u64::try_from(s.count).map_or(true, |n| n > logical_bytes))
        {
            return Err("保存済み復元ID件数が元論理データの上限を超えています".into());
        }
        let tables = portable::tables(&archive.database)?;
        let bindings = tables
            .iter()
            .map(|table| Ok((table.name.clone(), rules(&archive.database, table)?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let stage = archive::private_stage(parent)?;
        let path = stage.path().join("identities.sqlite3");
        private_file(&path)?;
        let database = Connection::open(path).map_err(|e| e.to_string())?;
        database.execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=FULL; PRAGMA cache_size=-8192;
            CREATE TABLE mappings(domain TEXT NOT NULL,scope TEXT NOT NULL,old_id TEXT NOT NULL,new_id TEXT NOT NULL,
                PRIMARY KEY(domain,scope,old_id));
            CREATE INDEX mappings_target ON mappings(new_id);
            CREATE TABLE reserved_ids(id TEXT PRIMARY KEY);
            CREATE TABLE issued_ids(id TEXT PRIMARY KEY);
            CREATE TABLE cached_mappings(domain TEXT NOT NULL,scope TEXT NOT NULL,old_id TEXT NOT NULL,new_id TEXT NOT NULL,PRIMARY KEY(domain,scope,old_id));
            CREATE TABLE publication_order(domain TEXT NOT NULL,old_id TEXT NOT NULL,position INTEGER NOT NULL,
                PRIMARY KEY(domain,old_id),UNIQUE(domain,position));
            CREATE TABLE metadata(batch_id TEXT NOT NULL,archive_id TEXT NOT NULL);
            BEGIN IMMEDIATE;") .map_err(|e| e.to_string())?;
        database
            .execute(
                "INSERT INTO metadata VALUES(?1,?2)",
                params![batch, archive.manifest.archive_id],
            )
            .map_err(|e| e.to_string())?;
        let plan = Self {
            database,
            source: archive,
            tables: tables.clone(),
            bindings,
            batch_id: batch.into(),
            archive_id: archive.manifest.archive_id.clone(),
            saved,
            stage,
        };
        plan.reserve(&plan.batch_id)?;
        plan.reserve(&plan.archive_id)?;
        if let Some(catalog) = &archive.manifest.model_catalog {
            for id in catalog.origin_ids() {
                check_cancelled(cancelled)?;
                plan.reserve(&id)?;
            }
        }
        plan.verify_draft_history(cancelled)?;
        plan.order_dependencies(cancelled)?;
        // Reserve every relational identifier before minting any new one. IDs
        // equal to a title/body are not identifiers and are not string-replaced.
        for table in &tables {
            let bindings = rules(&archive.database, table)?;
            for column in bindings.keys() {
                let mut query = archive
                    .database
                    .prepare(&format!(
                        "SELECT \"{column}\" FROM \"{}\" WHERE \"{column}\" IS NOT NULL",
                        table.name
                    ))
                    .map_err(|e| e.to_string())?;
                let mut rows = query.query([]).map_err(|e| e.to_string())?;
                while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    check_cancelled(cancelled)?;
                    plan.reserve(&row.get::<_, String>(0).map_err(|e| e.to_string())?)?;
                }
            }
        }
        // Draft-only and generated-original IDs may have no relational row.
        // Collect their definitions first; references are checked only afterward.
        plan.visit_documents(&archive.database, cancelled, 0)?;
        plan.visit_fixed_definitions(cancelled, 0)?;
        plan.visit_provenance(cancelled, false)?;
        for meeting in archive.deleted_audio.meetings() {
            plan.reserve(&super::restored_audio::operation_key(
                &plan.archive_id,
                meeting,
            ))?;
        }
        if let Some(destination) = destination {
            plan.check_destination_capacity(destination)?;
            plan.load_saved(destination, cancelled)?;
            if plan
                .saved
                .as_ref()
                .is_none_or(|s| s.state == super::ledger::BatchState::Prepared)
            {
                plan.reserve_destination(destination, cancelled)?;
            }
        }
        plan.seed_documents(&archive.database, cancelled)?;
        plan.visit_fixed_definitions(cancelled, 1)?;
        plan.seed_flow_operations(cancelled)?;
        plan.visit_provenance(cancelled, true)?;
        for table in &tables {
            let bindings = rules(&archive.database, table)?;
            let mut owners = archive
                .database
                .prepare(&owners_sql(&table.name)?)
                .map_err(|e| e.to_string())?;
            let mut rows = owners.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                let rowid: i64 = row.get(0).map_err(|e| e.to_string())?;
                let meeting: Option<String> = row.get(1).map_err(|e| e.to_string())?;
                let meeting = meeting.as_deref().unwrap_or("");
                for (column, rule) in &bindings {
                    // All references have existing definitions through FK checks,
                    // except historical exported draft IDs (intentionally not FK).
                    let declaration = column == "id"
                        || matches!(
                            rule,
                            Rule::SpeakerLineage
                                | Rule::UtteranceLineage
                                | Rule::Overlap
                                | Rule::Operation
                        )
                        || (table.name == "export_records" && column == "source_draft_id");
                    if !declaration {
                        continue;
                    }
                    let value: Option<String> = archive
                        .database
                        .query_row(
                            &format!("SELECT \"{column}\" FROM \"{}\" WHERE rowid=?1", table.name),
                            params![rowid],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    if let Some(value) = value {
                        if let Some(domain) = rule.domain() {
                            plan.declare(domain, scope(domain, meeting), &value)?;
                        }
                    }
                }
            }
        }
        // Native unavailable records need a stable, replayed operation identity.
        // A source operation using our synthetic key is a conflict, not authority.
        for meeting in archive.deleted_audio.meetings() {
            let key = super::restored_audio::operation_key(&plan.archive_id, meeting);
            if plan.lookup("operation", meeting, &key)?.is_some() {
                return Err("復元用の音声利用不可操作と元の操作IDが競合しています".into());
            }
            plan.declare("operation", meeting, &key)?;
        }
        // Validate every mapped reference, including nullable and composite FKs.
        for table in &tables {
            let bindings = rules(&archive.database, table)?;
            let mut owners = archive
                .database
                .prepare(&owners_sql(&table.name)?)
                .map_err(|e| e.to_string())?;
            let mut rows = owners.query([]).map_err(|e| e.to_string())?;
            let mut count = 0_i64;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                let rowid: i64 = row.get(0).map_err(|e| e.to_string())?;
                let meeting: Option<String> = row.get(1).map_err(|e| e.to_string())?;
                plan.mapped_row(
                    &archive.database,
                    table,
                    &bindings,
                    rowid,
                    meeting.as_deref().unwrap_or(""),
                )?;
                count += 1;
            }
            let actual: i64 = archive
                .database
                .query_row(&format!("SELECT COUNT(*) FROM {}", table.name), [], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            if count != actual {
                return Err("復元行の会議所属が欠損しています".into());
            }
        }
        plan.check_documents(&archive.database, cancelled)?;
        plan.verify_replay(cancelled)?;
        check_cancelled(cancelled)?;
        plan.database
            .execute_batch("COMMIT; PRAGMA query_only=ON;")
            .map_err(|e| e.to_string())?;
        File::open(plan.stage.path().join("identities.sqlite3"))
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(plan)
    }

    fn reserve(&self, old: &str) -> Result<(), String> {
        if old.is_empty() || old.len() > 256 || old.contains('\0') {
            return Err("復元識別子が不正です".into());
        }
        self.database
            .execute(
                "INSERT OR IGNORE INTO reserved_ids VALUES(?1)",
                params![old],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // Flow operations are derived identities, not independent random IDs. Mint
    // their owning flow first so fixed requests and normal handoff readers agree.
    fn seed_flow_operations(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let mut query = self
            .source
            .database
            .prepare("SELECT id,meeting_id FROM analysis_flows ORDER BY id")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let flow: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let new_flow = self.declare("analysis_flows", &meeting, &flow)?;
            for kind in ["transcription", "diarization"] {
                let old = format!("flow:{flow}:{kind}");
                let root: Option<(String,String,Option<String>,i64)> = self.source.database.query_row("SELECT meeting_id,kind,retry_of_job_id,attempt FROM processing_jobs WHERE operation_id=?1",params![old],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
                let Some((owner, actual_kind, parent, attempt)) = root else {
                    continue;
                };
                if owner != meeting || actual_kind != kind || parent.is_some() || attempt != 1 {
                    return Err("解析工程の初回操作が別会議・別工程または再試行です".into());
                }
                let new = format!("flow:{new_flow}:{kind}");
                if self.saved.is_some() && self.cached_target("operation", &meeting, &old)? != new {
                    return Err("保存済み解析工程の派生操作IDが一致しません".into());
                }
                self.reserve(&old)?;
                let conflict: bool = self.database.query_row("SELECT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1) OR EXISTS(SELECT 1 FROM issued_ids WHERE id=?1)",params![new],|r|r.get(0)).map_err(|e|e.to_string())?;
                if conflict || self.lookup("operation", &meeting, &old)?.is_some() {
                    return Err("解析工程の復元操作IDが競合しています".into());
                }
                self.database
                    .execute("INSERT INTO issued_ids VALUES(?1)", params![new])
                    .map_err(|e| e.to_string())?;
                self.database
                    .execute(
                        "INSERT INTO mappings VALUES('operation',?1,?2,?3)",
                        params![meeting, old, new],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    fn declare(&self, domain: &str, meeting: &str, old: &str) -> Result<String, String> {
        let meeting = scope(domain, meeting);
        self.reserve(old)?;
        if let Some(value) = self.lookup(domain, meeting, old)? {
            return Ok(value);
        }
        if self.saved.is_some() {
            let new = self.cached_target(domain, meeting, old)?;
            self.insert_cached(domain, meeting, old, &new)?;
            return Ok(new);
        }
        loop {
            let prefix = match domain {
                "speakers" if old.starts_with("manual:") => "manual:",
                "utterances" if old.starts_with("split:") => "split:",
                _ => "",
            };
            let new = format!("{prefix}{}", Uuid::new_v4());
            let reserved:bool=self.database.query_row("SELECT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1) OR EXISTS(SELECT 1 FROM issued_ids WHERE id=?1)",params![new],|r|r.get(0)).map_err(|e|e.to_string())?;
            if reserved {
                continue;
            }
            self.database
                .execute("INSERT INTO issued_ids VALUES(?1)", params![new])
                .map_err(|e| e.to_string())?;
            self.database
                .execute(
                    "INSERT INTO mappings VALUES(?1,?2,?3,?4)",
                    params![domain, meeting, old, new],
                )
                .map_err(|e| e.to_string())?;
            return Ok(new);
        }
    }

    fn lookup(&self, domain: &str, meeting: &str, old: &str) -> Result<Option<String>, String> {
        self.database
            .query_row(
                "SELECT new_id FROM mappings WHERE domain=?1 AND scope=?2 AND old_id=?3",
                params![domain, scope(domain, meeting), old],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn manual_identity(
        &self,
        domain: &str,
        lineage: &str,
        meeting: &str,
        old: &str,
    ) -> Result<(), String> {
        let new = self.declare(domain, meeting, old)?;
        if self.saved.is_some() && self.cached_target(lineage, meeting, old)? != new {
            return Err("保存済み手動編集と系譜の対応が一致しません".into());
        }
        self.database.execute("INSERT INTO mappings VALUES(?1,?2,?3,?4) ON CONFLICT(domain,scope,old_id) DO UPDATE SET new_id=excluded.new_id",params![lineage,meeting,old,new]).map_err(|e|e.to_string())?;
        Ok(())
    }

    pub(super) fn get(&self, domain: &str, meeting: &str, old: &str) -> Result<String, String> {
        self.lookup(domain, meeting, old)?
            .ok_or_else(|| format!("復元参照が未定義か別会議です: {domain}"))
    }

    pub(super) fn source(&self) -> &VerifiedArchive {
        self.source
    }

    pub(super) fn visit_mappings(
        &self,
        cancelled: &impl Fn() -> bool,
        mut visit: impl FnMut(super::ledger::IdentityMap) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut query = self
            .database
            .prepare("SELECT domain,scope,old_id,new_id FROM mappings ORDER BY domain,scope,old_id")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            visit(super::ledger::IdentityMap {
                domain: row.get(0).map_err(|e| e.to_string())?,
                scope: row.get(1).map_err(|e| e.to_string())?,
                source: row.get(2).map_err(|e| e.to_string())?,
                target: row.get(3).map_err(|e| e.to_string())?,
            })?;
        }
        check_cancelled(cancelled)
    }

    pub(super) fn unused_identity(&self, id: &str) -> Result<bool, String> {
        self.database.query_row("SELECT NOT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM issued_ids WHERE id=?1)",params![id],|r|r.get(0)).map_err(|e|e.to_string())
    }

    fn visit_provenance(&self, cancelled: &impl Fn() -> bool, declare: bool) -> Result<(), String> {
        if !self.tables.iter().any(|t| t.name == "restore_provenance") {
            return Ok(());
        }
        super::ledger::validate_provenance(&self.source.database, cancelled)?;
        let mut query=self.source.database.prepare("SELECT domain,meeting_id,entity_key,source_scope,source_key FROM restore_provenance ORDER BY id").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let domain: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: Option<String> = row.get(1).map_err(|e| e.to_string())?;
            let key: String = row.get(2).map_err(|e| e.to_string())?;
            if declare {
                self.declare(
                    &domain,
                    scope(&domain, meeting.as_deref().unwrap_or("")),
                    &key,
                )?;
            } else {
                self.reserve(&key)?;
                let old_scope: String = row.get(3).map_err(|e| e.to_string())?;
                if !old_scope.is_empty() {
                    self.reserve(&old_scope)?;
                }
                self.reserve(&row.get::<_, String>(4).map_err(|e| e.to_string())?)?;
            }
        }
        Ok(())
    }

    fn visit_fixed_definitions(
        &self,
        cancelled: &impl Fn() -> bool,
        phase: u8,
    ) -> Result<(), String> {
        super::restore_fixed::visit_definitions(
            &self.source.database,
            cancelled,
            &mut |definition| {
                if phase == 0 {
                    self.reserve(&definition.old)?;
                } else if let Some(lineage) = definition.lineage {
                    self.manual_identity(
                        definition.domain,
                        lineage,
                        &definition.meeting,
                        &definition.old,
                    )?;
                } else {
                    self.declare(definition.domain, &definition.meeting, &definition.old)?;
                }
                Ok(())
            },
        )
    }

    fn map_scope(&self, old: &str) -> Result<String, String> {
        if old == "global" {
            return Ok(old.into());
        }
        let meeting = old
            .strip_prefix("meeting:")
            .ok_or("復元用語スコープが不正です")?;
        Ok(format!("meeting:{}", self.get("meetings", "", meeting)?))
    }

    fn mapped_row(
        &self,
        source: &Connection,
        table: &Table,
        bindings: &BTreeMap<String, Rule>,
        rowid: i64,
        meeting: &str,
    ) -> Result<Vec<Value>, String> {
        source
            .query_row(
                &format!("SELECT * FROM \"{}\" WHERE rowid=?1", table.name),
                params![rowid],
                |row| {
                    (0..table.columns.len())
                        .map(|i| row.get::<_, Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                },
            )
            .map_err(|e| e.to_string())?
            .into_iter()
            .zip(&table.columns)
            .map(|(value, column)| {
                let Some(rule) = bindings.get(&column.name) else {
                    return Ok(value);
                };
                match value {
                    Value::Null => Ok(Value::Null),
                    Value::Text(old) => Ok(Value::Text(match rule {
                        Rule::VocabularyScope => self.map_scope(&old)?,
                        Rule::ProvenanceEntity => {
                            let domain: String = source
                                .query_row(
                                    "SELECT domain FROM restore_provenance WHERE rowid=?1",
                                    params![rowid],
                                    |r| r.get(0),
                                )
                                .map_err(|e| e.to_string())?;
                            self.get(&domain, meeting, &old)?
                        }
                        _ => self.get(
                            rule.domain().ok_or("復元ID領域がありません")?,
                            meeting,
                            &old,
                        )?,
                    })),
                    _ => Err("復元IDの列型が不正です".into()),
                }
            })
            .collect()
    }

    /// Only relational columns are mapped here. Do not insert these values into
    /// a live DB before rebuilding fixed JSON, file provenance and hashes.
    pub(super) fn relational_row(&self, table: &str, rowid: i64) -> Result<Vec<Value>, String> {
        let table = self
            .tables
            .iter()
            .find(|t| t.name == table)
            .ok_or("未知の復元テーブルです")?;
        let owner: Option<String> = self
            .source
            .database
            .query_row(
                &format!(
                    "SELECT * FROM ({}) WHERE rowid=?1",
                    owners_sql(&table.name)?
                ),
                params![rowid],
                |r| r.get(1),
            )
            .map_err(|e| e.to_string())?;
        self.mapped_row(
            &self.source.database,
            table,
            self.bindings
                .get(&table.name)
                .ok_or("復元ID列の対応がありません")?,
            rowid,
            owner.as_deref().unwrap_or(""),
        )
    }

    fn order_dependencies(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        for (domain,edge_sql) in [
            ("analysis_revisions","SELECT id,parent_revision_id FROM analysis_revisions WHERE parent_revision_id IS NOT NULL"),
            ("minutes_revisions","SELECT id,parent_revision_id FROM minutes_revisions WHERE parent_revision_id IS NOT NULL"),
            ("processing_jobs","SELECT id,retry_of_job_id FROM processing_jobs WHERE retry_of_job_id IS NOT NULL"),
            ("audio_tracks","SELECT derived_track_id,input_track_id FROM track_inputs"),
        ] {
            let mut nodes=BTreeMap::<String,usize>::new();
            let mut query=self.source.database.prepare(&format!("SELECT id FROM {domain}")).map_err(|e|e.to_string())?;
            let mut rows=query.query([]).map_err(|e|e.to_string())?;
            while let Some(row)=rows.next().map_err(|e|e.to_string())? {
                check_cancelled(cancelled)?;
                nodes.insert(row.get(0).map_err(|e|e.to_string())?,0);
            }
            let mut children=BTreeMap::<String,Vec<String>>::new();
            let mut query=self.source.database.prepare(edge_sql).map_err(|e|e.to_string())?;
            let mut rows=query.query([]).map_err(|e|e.to_string())?;
            let mut edges=std::collections::BTreeSet::new();
            while let Some(row)=rows.next().map_err(|e|e.to_string())? {
                check_cancelled(cancelled)?;
                let child:String=row.get(0).map_err(|e|e.to_string())?;
                let parent:String=row.get(1).map_err(|e|e.to_string())?;
                if !nodes.contains_key(&parent) || !nodes.contains_key(&child) { return Err("復元依存先が欠損しています".into()); }
                if edges.insert((child.clone(),parent.clone())) {
                    *nodes.get_mut(&child).ok_or("復元依存先が欠損しています")?+=1;
                    children.entry(parent).or_default().push(child);
                }
            }
            let mut ready:std::collections::BTreeSet<_>=nodes.iter().filter(|(_,degree)|**degree==0).map(|(id,_)|id.clone()).collect();
            let mut position=0_i64;
            while let Some(id)=ready.pop_first() {
                check_cancelled(cancelled)?;
                self.database.execute("INSERT INTO publication_order VALUES(?1,?2,?3)",params![domain,id,position]).map_err(|e|e.to_string())?;
                position+=1;
                for child in children.get(&id).into_iter().flatten() {
                    let degree=nodes.get_mut(child).ok_or("復元依存先が欠損しています")?;
                    *degree-=1;
                    if *degree==0 {ready.insert(child.clone());}
                }
            }
            if position as usize != nodes.len() {return Err(format!("復元依存参照が循環しています: {domain}"));}
        }
        Ok(())
    }
}

impl Rule {
    fn domain(self) -> Option<&'static str> {
        match self {
            Self::Entity(domain) => Some(domain),
            Self::SpeakerLineage => Some("speaker_lineage"),
            Self::UtteranceLineage => Some("utterance_lineage"),
            Self::Overlap => Some("overlap_group"),
            Self::Operation => Some("operation"),
            Self::VocabularyScope | Self::ProvenanceEntity => None,
        }
    }
}

impl RestoreIds<'_> {
    fn verify_draft_history(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let mut query=self.source.database.prepare("SELECT id,target_kind,payload_json,content_hash,schema_version FROM working_drafts ORDER BY id").map_err(|e|e.to_string())?;
        let mut drafts = query.query([]).map_err(|e| e.to_string())?;
        while let Some(draft) = drafts.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let id: String = draft.get(0).map_err(|e| e.to_string())?;
            let kind: String = draft.get(1).map_err(|e| e.to_string())?;
            let current: String = draft.get(2).map_err(|e| e.to_string())?;
            let hash: String = draft.get(3).map_err(|e| e.to_string())?;
            let schema: i64 = draft.get(4).map_err(|e| e.to_string())?;
            if current.len() > MAX_DRAFT_BYTES {
                return Err("復元下書きが容量上限を超えています".into());
            }
            let payload_schema = if kind == "transcript" {
                let payload: TranscriptDraftPayload =
                    serde_json::from_str(&current).map_err(|e| e.to_string())?;
                if !matches!(payload.schema_version, 1..=3)
                    || (payload.schema_version == 1
                        && (!payload.manual_speakers.is_empty()
                            || !payload.speaker_merges.is_empty()))
                    || (payload.schema_version < 3 && !payload.utterance_splits.is_empty())
                {
                    return Err("復元下書きの形式が不正です".into());
                }
                i64::from(payload.schema_version)
            } else {
                let payload: MinutesContent =
                    serde_json::from_str(&current).map_err(|e| e.to_string())?;
                if payload.schema_version != 1 {
                    return Err("復元下書きの形式が不正です".into());
                }
                i64::from(payload.schema_version)
            };
            if payload_schema != schema || draft_hash(&kind, &current)? != hash {
                return Err("復元下書きのハッシュ・形式が不正です".into());
            }
            let mut query=self.source.database.prepare("SELECT sequence,payload_json,inverse_json,before_hash,after_hash FROM edit_operations WHERE draft_id=?1 ORDER BY sequence").map_err(|e|e.to_string())?;
            let mut edits = query.query(params![id]).map_err(|e| e.to_string())?;
            let mut sequence = 0_i64;
            let mut previous: Option<String> = None;
            while let Some(edit) = edits.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                let next: i64 = edit.get(0).map_err(|e| e.to_string())?;
                let payload: String = edit.get(1).map_err(|e| e.to_string())?;
                let inverse: String = edit.get(2).map_err(|e| e.to_string())?;
                let before: String = edit.get(3).map_err(|e| e.to_string())?;
                let after: String = edit.get(4).map_err(|e| e.to_string())?;
                if payload.len() > MAX_DRAFT_BYTES
                    || inverse.len() > MAX_DRAFT_BYTES
                    || sequence.checked_add(1) != Some(next)
                    || draft_hash(&kind, &payload)? != after
                    || draft_hash(&kind, &inverse)? != before
                    || previous.as_ref().is_some_and(|p| p != &before)
                {
                    return Err("復元編集履歴のハッシュ・つながりが不正です".into());
                }
                sequence = next;
                previous = Some(after);
            }
            if previous.as_ref().is_some_and(|last| last != &hash) {
                return Err("復元編集履歴と下書きが一致しません".into());
            }
        }
        Ok(())
    }

    pub(super) fn draft_json(
        &self,
        kind: &str,
        meeting: &str,
        text: &str,
    ) -> Result<(String, String), String> {
        if text.len() > MAX_DRAFT_BYTES {
            return Err("復元下書きが容量上限を超えています".into());
        }
        let text = match kind {
            "transcript" => {
                let mut payload: TranscriptDraftPayload =
                    serde_json::from_str(text).map_err(|e| e.to_string())?;
                self.transcript(meeting, &mut payload)?;
                serde_json::to_string(&payload).map_err(|e| e.to_string())?
            }
            "minutes" => {
                let mut payload: MinutesContent =
                    serde_json::from_str(text).map_err(|e| e.to_string())?;
                self.minutes(meeting, &mut payload)?;
                serde_json::to_string(&payload).map_err(|e| e.to_string())?
            }
            _ => return Err("未知の復元下書きです".into()),
        };
        if text.len() > MAX_DRAFT_BYTES {
            return Err("再構成した下書きが容量上限を超えています".into());
        }
        let hash = draft_hash(kind, &text)?;
        Ok((text, hash))
    }

    /// Rebuild drafts and their complete undo chain only. These rows cannot be
    /// published before their base revisions and fixed inputs are reconstructed.
    pub(super) fn draft_row(&self, table: &str, rowid: i64) -> Result<Vec<Value>, String> {
        let sql=match table {
            "working_drafts"=>"SELECT meeting_id,target_kind FROM working_drafts WHERE rowid=?1",
            "edit_operations"=>"SELECT d.meeting_id,d.target_kind FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id WHERE e.rowid=?1",
            _=>return Err("復元下書き以外の行です".into()),
        };
        let (meeting, kind): (String, String) = self
            .source
            .database
            .query_row(sql, params![rowid], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?;
        let columns = &self
            .tables
            .iter()
            .find(|t| t.name == table)
            .ok_or("復元下書き列がありません")?
            .columns;
        let mut row = self.relational_row(table, rowid)?;
        let index = |name: &str| {
            columns
                .iter()
                .position(|c| c.name == name)
                .ok_or("復元下書き列がありません")
        };
        for (json, hash) in if table == "working_drafts" {
            vec![("payload_json", "content_hash")]
        } else {
            vec![
                ("payload_json", "after_hash"),
                ("inverse_json", "before_hash"),
            ]
        } {
            let i = index(json)?;
            let text = match &row[i] {
                Value::Text(t) => t,
                _ => return Err("復元下書きの列型が不正です".into()),
            };
            let (text, digest) = self.draft_json(&kind, &meeting, text)?;
            row[i] = Value::Text(text);
            row[index(hash)?] = Value::Text(digest);
        }
        Ok(row)
    }

    fn visit_documents(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
        phase: u8,
    ) -> Result<(), String> {
        for sql in [
            "SELECT meeting_id,target_kind,payload_json FROM working_drafts",
            "SELECT d.meeting_id,d.target_kind,e.payload_json FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id",
            "SELECT d.meeting_id,d.target_kind,e.inverse_json FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id",
            "SELECT meeting_id,'minutes',generated_document_json FROM minutes_revisions",
            "SELECT meeting_id,'minutes',effective_document_json FROM minutes_revisions",
        ] {
            let mut query=db.prepare(sql).map_err(|e|e.to_string())?;
            let mut rows=query.query([]).map_err(|e|e.to_string())?;
            while let Some(row)=rows.next().map_err(|e|e.to_string())? {
                check_cancelled(cancelled)?;
                let meeting:String=row.get(0).map_err(|e|e.to_string())?;
                let kind:String=row.get(1).map_err(|e|e.to_string())?;
                let text:String=row.get(2).map_err(|e|e.to_string())?;
                if kind=="transcript" {
                    let mut payload:TranscriptDraftPayload=serde_json::from_str(&text).map_err(|e|e.to_string())?;
                    if phase==0 {
                        for s in &payload.manual_speakers { self.reserve(&s.id)?; }
                        for p in payload.utterance_splits.iter().flat_map(|s|&s.parts) { self.reserve(&p.id)?; }
                    } else if phase==1 {
                        for s in &payload.manual_speakers { self.manual_identity("speakers","speaker_lineage",&meeting,&s.id)?; }
                        for p in payload.utterance_splits.iter().flat_map(|s|&s.parts) { self.manual_identity("utterances","utterance_lineage",&meeting,&p.id)?; }
                    } else { self.transcript(&meeting,&mut payload)?; }
                } else {
                    let mut content:MinutesContent=serde_json::from_str(&text).map_err(|e|e.to_string())?;
                    if phase==0 {
                        for item in &content.items { self.reserve(&item.id)?; }
                        for evidence in &content.evidence { self.reserve(&evidence.id)?; }
                    } else if phase==1 {
                        for item in &content.items { self.declare("minutes_items",&meeting,&item.id)?; }
                        for evidence in &content.evidence { self.declare("evidence_links",&meeting,&evidence.id)?; }
                    } else { self.minutes(&meeting,&mut content)?; }
                }
            }
        }
        Ok(())
    }
    fn seed_documents(&self, db: &Connection, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.visit_documents(db, cancelled, 1)
    }
    fn check_documents(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.visit_documents(db, cancelled, 2)
    }

    pub(super) fn transcript(
        &self,
        meeting: &str,
        payload: &mut TranscriptDraftPayload,
    ) -> Result<(), String> {
        let mut mapped = payload.clone();
        self.transcript_inner(meeting, &mut mapped)?;
        *payload = mapped;
        Ok(())
    }

    fn transcript_inner(
        &self,
        meeting: &str,
        payload: &mut TranscriptDraftPayload,
    ) -> Result<(), String> {
        for edit in &mut payload.speaker_names {
            edit.speaker_id = self.get("speakers", meeting, &edit.speaker_id)?;
        }
        for edit in &mut payload.utterances {
            edit.utterance_id = self.get("utterances", meeting, &edit.utterance_id)?;
            for id in &mut edit.speaker_ids {
                *id = self.get("speakers", meeting, id)?;
            }
        }
        for speaker in &mut payload.manual_speakers {
            speaker.id = self.get("speakers", meeting, &speaker.id)?;
        }
        for merge in &mut payload.speaker_merges {
            merge.source_id = self.get("speakers", meeting, &merge.source_id)?;
            merge.target_id = self.get("speakers", meeting, &merge.target_id)?;
        }
        for split in &mut payload.utterance_splits {
            split.source_utterance_id =
                self.get("utterances", meeting, &split.source_utterance_id)?;
            for part in &mut split.parts {
                part.id = self.get("utterances", meeting, &part.id)?;
                for id in &mut part.speaker_ids {
                    *id = self.get("speakers", meeting, id)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn minutes(
        &self,
        meeting: &str,
        content: &mut MinutesContent,
    ) -> Result<(), String> {
        let mut mapped = content.clone();
        self.minutes_inner(meeting, &mut mapped)?;
        *content = mapped;
        Ok(())
    }

    fn minutes_inner(&self, meeting: &str, content: &mut MinutesContent) -> Result<(), String> {
        let item = |id: &mut Option<String>| -> Result<(), String> {
            if let Some(old) = id {
                *old = self.get("minutes_items", meeting, old)?;
            }
            Ok(())
        };
        let inline = |parts: &mut Vec<Inline>| -> Result<(), String> {
            for part in parts {
                if let Inline::Evidence {
                    minutes_item_id,
                    evidence_link_id,
                } = part
                {
                    *minutes_item_id = self.get("minutes_items", meeting, minutes_item_id)?;
                    *evidence_link_id = self.get("evidence_links", meeting, evidence_link_id)?;
                }
            }
            Ok(())
        };
        for block in &mut content.blocks {
            match block {
                Block::Heading {
                    item_id, content, ..
                }
                | Block::Paragraph { item_id, content } => {
                    item(item_id)?;
                    inline(content)?;
                }
                Block::List { items, .. } => {
                    for row in items {
                        item(&mut row.item_id)?;
                        inline(&mut row.content)?;
                    }
                }
                Block::Table { headers, rows } => {
                    for header in headers {
                        inline(header)?;
                    }
                    for row in rows {
                        item(&mut row.item_id)?;
                        for cell in &mut row.cells {
                            inline(cell)?;
                        }
                    }
                }
            }
        }
        for i in &mut content.items {
            i.id = self.get("minutes_items", meeting, &i.id)?;
            if let Some(id) = &mut i.assignee_speaker_id {
                *id = self.get("speakers", meeting, id)?;
            }
        }
        for e in &mut content.evidence {
            e.id = self.get("evidence_links", meeting, &e.id)?;
            e.minutes_item_id = self.get("minutes_items", meeting, &e.minutes_item_id)?;
            e.analysis_revision_id =
                self.get("analysis_revisions", meeting, &e.analysis_revision_id)?;
            e.utterance_id = self.get("utterances", meeting, &e.utterance_id)?;
        }
        Ok(())
    }
}

fn draft_hash(kind: &str, text: &str) -> Result<String, String> {
    match kind {
        "transcript" => Ok(super::super::drafts::draft_hash(text)),
        "minutes" => Ok(super::super::minutes_drafts::hash(text)),
        _ => Err("未知の復元下書きです".into()),
    }
}

#[cfg(test)]
#[path = "restore_ids_tests.rs"]
pub(super) mod tests;
