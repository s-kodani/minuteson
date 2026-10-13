//! File-backed reconstruction; only seal::SealedWorkspace can pass to publication.
//! Saved artifacts and processing history have consuming construction passes.
//! Durable provenance and final sealing do not themselves publish live meetings.
use super::*;
use std::io::Read;
mod assets;
mod generation;
mod history;
mod provenance;
pub(in crate::storage::backup) mod seal;
mod speech;

pub(in crate::storage::backup) struct FixedWorkspace {
    // Close SQLite before removing only this operation's private directory.
    database: Connection,
    stage: scratch::Stage,
    // Definitions come from our migrations, not from archived executable SQL.
    triggers: Vec<String>,
    missing_assets: std::collections::BTreeSet<String>,
    completed: [bool; 4],
    final_database_proof: Option<(u64, String)>,
}

impl FixedWorkspace {
    pub(in crate::storage::backup) fn with_provenance(
        mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if self.completed != [true, true, true, false] {
            return Err("復元成果物と処理履歴の構築が完了していません".into());
        }
        self.provenance(fixed, cancelled)?;
        self.completed[3] = true;
        self.final_database_proof = Some(digest_file(
            &self.stage.path().join("fixed.sqlite3"),
            cancelled,
        )?);
        Ok(self)
    }
    pub(in crate::storage::backup) fn with_processing_history(
        mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if !self.completed[0] || !self.completed[1] {
            return Err("音声・生成成果物の構築後に復元処理履歴を構築してください".into());
        }
        if self.completed[2] || self.completed[3] {
            return Err("復元処理履歴の構築が既に完了しています".into());
        }
        self.processing_history(fixed, cancelled)?;
        self.completed[2] = true;
        Ok(self)
    }
    pub(in crate::storage::backup) fn with_speech_artifacts(
        mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if self.completed[0] || self.completed[2] || self.completed[3] {
            return Err("復元音声成果物の構築が既に完了しています".into());
        }
        self.speech_artifacts(fixed, cancelled)?;
        self.completed[0] = true;
        Ok(self)
    }
    // Consumes an unsealed workspace: an error also closes SQLite and removes
    // this operation's partial files. No live DB or available model is exposed.
    pub(in crate::storage::backup) fn with_generation_artifacts(
        mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if self.completed[1] || self.completed[2] || self.completed[3] {
            return Err("復元生成成果物の構築が既に完了しています".into());
        }
        self.generation_artifacts(fixed, cancelled)?;
        self.completed[1] = true;
        Ok(self)
    }

    pub(in crate::storage::backup) fn prepare(
        fixed: &FixedData<'_, '_>,
        parent: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let source = fixed.ids.source();
        let rows = source.manifest.tables.iter().try_fold(0_u64, |sum, t| {
            sum.checked_add(t.row_count).ok_or("復元行数が範囲外です")
        })?;
        let bytes = source.manifest.entries.iter().try_fold(0_u64, |sum, e| {
            sum.checked_add(e.size)
                .ok_or("復元ファイル容量が範囲外です")
        })?;
        check_capacity(
            parent,
            bytes
                .checked_add(rows.checked_mul(4096).ok_or("復元DB容量が範囲外です")?)
                .ok_or("復元容量が範囲外です")?,
        )?;
        let stage = archive::private_stage(parent)?;
        let image = stage.path().join("fixed.sqlite3");
        private_file(&image)?;
        let database = Connection::open(&image).map_err(|e| e.to_string())?;
        database
            .execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=FULL;")
            .map_err(|e| e.to_string())?;
        let database = portable::initialize(database, MIGRATIONS.len())?;
        let definitions: Vec<(String, String)> = database
            .prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' ORDER BY name")
            .map_err(|e| e.to_string())?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (name, _) in &definitions {
            // Trusted schema names are still quoted, rather than interpolating
            // any archive-supplied identifier or CREATE statement.
            database
                .execute_batch(&format!("DROP TRIGGER \"{}\";", name.replace('"', "\"\"")))
                .map_err(|e| e.to_string())?;
        }
        let mut work = Self {
            database,
            stage,
            triggers: definitions.into_iter().map(|(_, sql)| sql).collect(),
            missing_assets: std::collections::BTreeSet::new(),
            completed: [false; 4],
            final_database_proof: None,
        };
        work.load_rows(fixed, cancelled)?;
        work.foundation_assets(fixed, cancelled)?;
        work.rebuild_dependencies(fixed, cancelled)?;
        check_cancelled(cancelled)?;
        Ok(work)
    }

    fn load_rows(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let tx = self.database.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("PRAGMA defer_foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        for table in portable::tables(&fixed.ids.source().database)? {
            check_cancelled(cancelled)?;
            let columns = table
                .columns
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "INSERT INTO \"{}\"({columns}) VALUES({})",
                table.name,
                vec!["?"; table.columns.len()].join(",")
            );
            let mut insert = tx.prepare(&sql).map_err(|e| e.to_string())?;
            let mut query = fixed
                .ids
                .source()
                .database
                .prepare(&format!(
                    "SELECT rowid FROM \"{}\" ORDER BY rowid",
                    table.name
                ))
                .map_err(|e| e.to_string())?;
            let mut rows = query.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                insert
                    .execute(rusqlite::params_from_iter(
                        fixed.typed_row(&table, row.get(0).map_err(|e| e.to_string())?)?,
                    ))
                    .map_err(|e| e.to_string())?;
            }
        }
        check_cancelled(cancelled)?;
        tx.commit().map_err(|e| e.to_string())
    }

    fn foundation_assets(
        &self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let source = fixed.ids.source();
        if !source.deleted_audio.is_empty() {
            source.verify_validation_database(cancelled)?;
        }
        source
            .deleted_audio
            .verify_omissions(&source.manifest.omitted_assets)?;
        let entries: std::collections::BTreeMap<_, _> = source
            .manifest
            .entries
            .iter()
            .filter_map(|e| e.asset_id.as_deref().map(|id| (id, e)))
            .collect();
        let mut query = source.database.prepare(
            "SELECT f.id,f.meeting_id,f.media_type,f.size_bytes,f.content_hash,
             EXISTS(SELECT 1 FROM audio_import_commits p WHERE p.manifest_asset_id=f.id),
             EXISTS(SELECT 1 FROM audio_chunks c WHERE c.file_asset_id=f.id)
               OR EXISTS(SELECT 1 FROM audio_imports i WHERE i.original_asset_id=f.id OR i.converted_asset_id=f.id)
               OR EXISTS(SELECT 1 FROM recording_sessions r WHERE r.meeting_id=f.meeting_id AND r.audio_manifest_hash=f.content_hash AND f.media_type='application/json')
             FROM file_assets f ORDER BY f.meeting_id,f.id",
        ).map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let new = fixed.ids.get("file_assets", &meeting, &id)?;
            let relative = format!(
                "meetings/{}/restored/{new}",
                fixed.ids.get("meetings", "", &meeting)?
            );
            self.database
                .execute(
                    "UPDATE file_assets SET relative_path=?1 WHERE id=?2",
                    params![relative, new],
                )
                .map_err(|e| e.to_string())?;
            let import_manifest: bool = row.get(5).map_err(|e| e.to_string())?;
            let foundation: bool = row.get(6).map_err(|e| e.to_string())?;
            if source.deleted_audio.asset(&meeting, &id).is_some() {
                fixed.absent_source_asset(&meeting, &id)?;
                // Seal walks all minted parents, including missing assets. Make
                // only private owned directories, never dummy audio bodies.
                let destination = self.stage.path().join(&relative);
                scratch::directories(destination.parent().ok_or("復元先フォルダーがありません")?)?;
                continue;
            }
            if !import_manifest && !foundation {
                // Includes omitted PCM/logs and durable native results. They
                // must be handled by their codecs before any seal/publication.
                continue;
            }
            // Resolve only the known owned root's platform alias. Keep the
            // generated relative path so copy_asset still rejects leaf/parent
            // symlinks and replacement, rather than following a substituted file.
            let input = source.owned_asset_path(&id)?;
            let destination = self.stage.path().join(&relative);
            let directory = destination.parent().ok_or("復元先フォルダーがありません")?;
            scratch::directories(directory)?;
            // Archive payload hashes can differ from source hashes after
            // portability redaction. Check the verified physical payload first.
            let entry = entries
                .get(id.as_str())
                .ok_or("復元元ファイルのmanifestがありません")?;
            if digest_file(&input, cancelled)? != (entry.size, entry.sha256.clone()) {
                return Err("検査済み復元ファイルが変更されています".into());
            }
            let (size, hash) = if import_manifest {
                if entry.size > portable::MAX_ROW_BYTES as u64 {
                    return Err("取り込みmanifestが容量上限を超えています".into());
                }
                let mut encoded = Vec::new();
                open_source(&input)?
                    .0
                    .take(portable::MAX_ROW_BYTES as u64 + 1)
                    .read_to_end(&mut encoded)
                    .map_err(|e| e.to_string())?;
                if encoded.len() as u64 != entry.size
                    || format!("{:x}", Sha256::digest(&encoded)) != entry.sha256
                {
                    return Err("取り込みmanifestの読込中に内容が変更されています".into());
                }
                check_cancelled(cancelled)?;
                let original: Value =
                    serde_json::from_slice(&encoded).map_err(|e| e.to_string())?;
                let bytes = serde_json::to_vec(&fixed.import_manifest(&meeting, &original)?)
                    .map_err(|e| e.to_string())?;
                let mut output = private_file(&destination)?;
                output.write_all(&bytes).map_err(|e| e.to_string())?;
                output.sync_all().map_err(|e| e.to_string())?;
                fixed.asset_bytes(&meeting, &id, &bytes)?
            } else {
                let payload = FileAsset {
                    id: id.clone(),
                    meeting_id: meeting.clone(),
                    relative_path: relative.clone(),
                    media_type: row.get(2).map_err(|e| e.to_string())?,
                    size_bytes: i64::try_from(entry.size)
                        .map_err(|_| "復元音声容量が範囲外です")?,
                    content_hash: entry.sha256.clone(),
                    state: "ready".into(),
                };
                copy_asset(&input, &destination, &payload, cancelled)?;
                fixed.asset_file(&meeting, &id, &destination, cancelled)?
            };
            self.database
                .execute(
                    "UPDATE file_assets SET size_bytes=?1,content_hash=?2 WHERE id=?3",
                    params![
                        i64::try_from(size).map_err(|_| "復元音声容量が範囲外です")?,
                        hash,
                        new
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        let mut query = source
            .database
            .prepare("SELECT meeting_id,manifest_asset_id,manifest_hash FROM audio_import_commits")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let meeting: String = row.get(0).map_err(|e| e.to_string())?;
            let asset: String = row.get(1).map_err(|e| e.to_string())?;
            let old: String = row.get(2).map_err(|e| e.to_string())?;
            self.database
                .execute(
                    "UPDATE audio_import_commits SET manifest_hash=?1 WHERE meeting_id=?2",
                    params![
                        fixed.resolve("file_assets", &meeting, &asset, &old)?,
                        fixed.ids.get("meetings", "", &meeting)?
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        // Recording manifests have been redacted, but their portable bytes have
        // no application-ID rewrite in this pass. Rebind their physical SHA.
        let mut query=source.database.prepare("SELECT id,meeting_id,audio_manifest_hash FROM recording_sessions WHERE audio_manifest_hash IS NOT NULL").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: String = row.get(1).map_err(|e| e.to_string())?;
            let old: String = row.get(2).map_err(|e| e.to_string())?;
            self.database
                .execute(
                    "UPDATE recording_sessions SET audio_manifest_hash=?1 WHERE id=?2",
                    params![
                        fixed.audio_manifest_hash(&meeting, &old)?,
                        fixed.ids.get("recording_sessions", &meeting, &id)?
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        let mut query = source
            .database
            .prepare("SELECT id,meeting_id FROM vocabulary_scopes")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let old: String = row.get(0).map_err(|e| e.to_string())?;
            let meeting: Option<String> = row.get(1).map_err(|e| e.to_string())?;
            let new = match meeting {
                Some(m) => format!("meeting:{}", fixed.ids.get("meetings", "", &m)?),
                None => "global".into(),
            };
            let hash = fixed.vocabulary_book_hash(&self.database, &old)?;
            self.database
                .execute(
                    "UPDATE vocabulary_scopes SET content_hash=?1 WHERE id=?2",
                    params![hash, new],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn rebuild_dependencies(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let mut position = 0_i64;
        while let Some(step) = fixed.ordered_step(position)? {
            check_cancelled(cancelled)?;
            let id = fixed.ids.get(&step.domain, &step.meeting, &step.old_id)?;
            match step.domain.as_str() {
                "analysis_revisions" => {
                    let (parent, old_parent, audio):(Option<String>,Option<String>,String)=fixed.ids.source().database.query_row("SELECT parent_revision_id,parent_content_hash,audio_manifest_hash FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",params![step.old_id,step.meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
                    let parent = parent
                        .as_deref()
                        .map(|p| {
                            fixed.resolve(
                                "analysis_revisions",
                                &step.meeting,
                                p,
                                old_parent.as_deref().ok_or("親版ハッシュがありません")?,
                            )
                        })
                        .transpose()?;
                    let (config, snapshot) =
                        fixed.analysis_config(&step.meeting, &step.old_id, &self.database)?;
                    let tx = self.database.transaction().map_err(|e| e.to_string())?;
                    tx.execute("UPDATE analysis_revisions SET config_json=?1,parent_content_hash=?2,audio_manifest_hash=?3 WHERE id=?4",params![config.to_string(),parent,fixed.audio_manifest_hash(&step.meeting,&audio)?,id]).map_err(|e|e.to_string())?;
                    if let Some(snapshot) = snapshot {
                        let draft = snapshot
                            .source_draft
                            .as_ref()
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(|e| e.to_string())?;
                        tx.execute("UPDATE analysis_input_snapshots SET source_content_hash=?1,source_draft_json=?2,working_content_hash=?3 WHERE analysis_id=?4",params![snapshot.source_content_hash,draft,snapshot.working_content_hash,id]).map_err(|e|e.to_string())?;
                    }
                    let hash = fixed.remember_revision(
                        &tx,
                        "analysis_revisions",
                        &step.meeting,
                        &step.old_id,
                    )?;
                    tx.execute(
                        "UPDATE analysis_revisions SET content_hash=?1 WHERE id=?2",
                        params![hash, id],
                    )
                    .map_err(|e| e.to_string())?;
                    tx.commit().map_err(|e| e.to_string())?;
                }
                "processing_jobs" => {
                    let (retry,attempt):(Option<String>,i64)=fixed.ids.source().database.query_row("SELECT retry_of_job_id,attempt FROM processing_jobs WHERE id=?1 AND meeting_id=?2",params![step.old_id,step.meeting],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
                    let retry = retry
                        .as_deref()
                        .map(|r| fixed.ids.get("processing_jobs", &step.meeting, r))
                        .transpose()?;
                    let spec = fixed.job_spec(&step.meeting, &step.old_id)?;
                    let hash = fixed.job_hash(
                        &step.meeting,
                        &step.old_id,
                        &spec,
                        retry.as_deref(),
                        attempt,
                    )?;
                    self.database.execute("UPDATE processing_jobs SET input_hash=?1,profile_snapshot_json=?2,parameters_json=?3,spec_hash=?4 WHERE id=?5",params![spec.input_hash,spec.profile_snapshot.to_string(),spec.parameters.to_string(),hash,id]).map_err(|e|e.to_string())?;
                }
                "minutes_revisions" => {
                    return Err("旧形式の構造化議事録には対応していません".into())
                }
                _ => return Err("未知の固定データ再構成段階です".into()),
            }
            position = position.checked_add(1).ok_or("復元段階数が範囲外です")?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
