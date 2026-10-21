//! One-time development reset: preserve captures, model settings and voice book.
use super::*;
use std::collections::BTreeSet;

pub(super) fn initialize(db: &mut Connection, root: &Path) -> Result<(), String> {
    let initialized: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key='meeting_mix_v1_initialized')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !initialized {
        let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if pending {
            return Err("会議音声方式の初期化には録音の停止が必要です".into());
        }
        let tables: BTreeSet<&str> = [
            "utterance_speakers",
            "evidence_links",
            "minutes_items",
            "voice_matches",
            "diarization_manual_assignments",
            "edit_operations",
            "working_drafts",
            "transcript_edit_recovery",
            "export_records",
            "job_checkpoints",
            "vocabulary_snapshots",
            "analysis_flows",
            "minutes_markdown",
            "processing_jobs",
            "analysis_input_snapshots",
            "minutes_revisions",
            "utterances",
            "speaker_segments",
            "speakers",
            "analysis_revisions",
        ]
        .into_iter()
        .collect();
        let triggers: Vec<(String, String)> = {
            let mut q = db
                .prepare(
                    "SELECT name,sql FROM sqlite_master WHERE type='trigger' AND sql IS NOT NULL",
                )
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        db.execute_batch("PRAGMA foreign_keys=OFF;")
            .map_err(|e| e.to_string())?;
        let reset = (|| {
            let tx = db.transaction().map_err(|e| e.to_string())?;
            for (name, _) in &triggers {
                tx.execute_batch(&format!("DROP TRIGGER \"{}\";", name.replace('"', "\"\"")))
                    .map_err(|e| e.to_string())?;
            }
            let mut paths: Vec<String> = {
                let mut q=tx.prepare("SELECT f.relative_path FROM file_assets f WHERE (f.relative_path LIKE 'meetings/%/analysis/%' OR f.relative_path LIKE 'meetings/%/analysis-input/%' OR EXISTS(SELECT 1 FROM audio_chunks c JOIN audio_tracks t ON t.id=c.track_id WHERE c.file_asset_id=f.id AND t.kind='processed')) AND NOT EXISTS(SELECT 1 FROM audio_chunks c JOIN audio_tracks t ON t.id=c.track_id WHERE c.file_asset_id=f.id AND t.kind!='processed') AND NOT EXISTS(SELECT 1 FROM audio_imports i WHERE i.original_asset_id=f.id OR i.converted_asset_id=f.id)").map_err(|e|e.to_string())?;
                let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
            };
            // Failed or cancelled workers may have written files before asset
            // registration. These two namespaces contain derived audio only.
            let meetings: Vec<String> = {
                let mut q = tx
                    .prepare("SELECT id FROM meetings")
                    .map_err(|e| e.to_string())?;
                let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
            };
            for meeting in meetings {
                if Path::new(&meeting).components().count() != 1
                    || !matches!(
                        Path::new(&meeting).components().next(),
                        Some(Component::Normal(_))
                    )
                {
                    return Err("解析初期化の会議IDが不正です".into());
                }
                for namespace in ["analysis", "analysis-input"] {
                    collect_derived_files(
                        root,
                        &root.join("meetings").join(&meeting).join(namespace),
                        &mut paths,
                    )?;
                }
            }
            let protected: BTreeSet<String> = {
                let mut q = tx.prepare("SELECT f.relative_path FROM file_assets f WHERE EXISTS(SELECT 1 FROM audio_chunks c JOIN audio_tracks t ON t.id=c.track_id WHERE c.file_asset_id=f.id AND t.kind!='processed') OR EXISTS(SELECT 1 FROM audio_imports i WHERE i.original_asset_id=f.id OR i.converted_asset_id=f.id)").map_err(|e| e.to_string())?;
                let rows = q.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
            };
            paths.retain(|path| !protected.contains(path));
            paths.sort();
            paths.dedup();
            let value = serde_json::to_string(&paths).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('meeting_mix_v1_cleanup',?1,?2,1,?3)",params![value,format!("{:x}",Sha256::digest(value.as_bytes())),now_ms()?]).map_err(|e|e.to_string())?;
            tx.execute_batch("UPDATE meetings SET current_analysis_id=NULL,previous_analysis_id=NULL,current_minutes_id=NULL,previous_minutes_id=NULL;
                UPDATE recording_sessions SET capture_config_json=json_remove(capture_config_json,'$.speakerCounts');
                UPDATE voice_samples SET source_analysis_id=NULL,source_track_id=CASE WHEN source_track_id IN (SELECT id FROM audio_tracks WHERE kind='processed') THEN NULL ELSE source_track_id END;
").map_err(|e|e.to_string())?;
            tx.execute_batch("DELETE FROM track_inputs WHERE derived_track_id IN (SELECT id FROM audio_tracks WHERE kind='processed') OR input_track_id IN (SELECT id FROM audio_tracks WHERE kind='processed');
                DELETE FROM audio_gaps WHERE track_id IN (SELECT id FROM audio_tracks WHERE kind='processed');
                DELETE FROM audio_chunks WHERE track_id IN (SELECT id FROM audio_tracks WHERE kind='processed');
                DELETE FROM track_clock_segments WHERE track_id IN (SELECT id FROM audio_tracks WHERE kind='processed');
                DELETE FROM audio_tracks WHERE kind='processed';").map_err(|e|e.to_string())?;
            for table in tables {
                tx.execute_batch(&format!("DELETE FROM {table};"))
                    .map_err(|e| e.to_string())?;
            }
            tx.execute("DELETE FROM restore_omitted_assets WHERE asset_id IN (SELECT id FROM file_assets WHERE relative_path IN (SELECT value FROM json_each(?1)))",[&value]).map_err(|e|e.to_string())?;
            tx.execute(
                "DELETE FROM file_assets WHERE relative_path IN (SELECT value FROM json_each(?1))",
                [&value],
            )
            .map_err(|e| e.to_string())?;
            for (_, sql) in &triggers {
                tx.execute_batch(sql).map_err(|e| e.to_string())?;
            }
            let violation = tx
                .prepare("PRAGMA foreign_key_check")
                .map_err(|e| e.to_string())?
                .exists([])
                .map_err(|e| e.to_string())?;
            if violation {
                return Err("初期化後の参照整合性を確認できません".into());
            }
            let value = "{\"schemaVersion\":1}";
            tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('meeting_mix_v1_initialized',?1,?2,1,?3)",params![value,format!("{:x}",Sha256::digest(value.as_bytes())),now_ms()?]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())
        })();
        db.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        reset?;
    }
    let pending: Option<String> = db
        .query_row(
            "SELECT value_json FROM app_settings WHERE key='meeting_mix_v1_cleanup'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let paths: Vec<String> = pending
        .map(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_default();
    for relative in paths {
        if Path::new(&relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("解析初期化のパスが不正です".into());
        }
        let path = root.join(&relative);
        if path.exists() {
            let actual = path.canonicalize().map_err(|e| e.to_string())?;
            if actual != path || !actual.starts_with(root) {
                return Err("解析初期化のファイル実体が不正です".into());
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    db.execute(
        "DELETE FROM app_settings WHERE key='meeting_mix_v1_cleanup'",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// Record exact files before removing DB references. Never follow a link or
// recursively remove a directory: interruption can safely replay the queue.
fn collect_derived_files(root: &Path, path: &Path, files: &mut Vec<String>) -> Result<(), String> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if metadata.file_type().is_symlink() || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("解析初期化の領域にシンボリックリンクがあります".into());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            collect_derived_files(root, &entry.map_err(|e| e.to_string())?.path(), files)?;
        }
    } else if metadata.is_file() {
        files.push(
            path.strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("解析初期化のパスが不正です")?
                .to_owned(),
        );
    } else {
        return Err("解析初期化のファイル種別が不正です".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn reset_preserves_original_audio_and_voice_book_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let (s, m, _) = super::super::transcription::tests::fixture(root.path(), 4);
        let mixed = s.ensure_meeting_mix(&m, &|| false, &|_, _| {}).unwrap();
        s.ensure_analysis_flow(&m).unwrap();
        let partial = s
            .root()
            .join("meetings")
            .join(&m)
            .join("analysis")
            .join("unregistered");
        std::fs::create_dir_all(&partial).unwrap();
        let partial = partial.join("input.wav");
        std::fs::write(&partial, b"cancelled worker").unwrap();
        let mut db = s.connection.lock().unwrap();
        db.execute(
            "INSERT INTO voice_people(id,name,created_at) VALUES ('person','登録人物',0)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO voice_samples(id,person_id,model_hash,extractor_version,dimension,vector,source_meeting_id,source_analysis_id,source_track_id,start_us,end_us,created_at) VALUES ('voice','person',?1,?5,2,?2,?3,'old-analysis',?4,0,3000000,0)",params!["a".repeat(64),crate::voice::encode(&[1.,0.]).unwrap(),m,mixed.tracks[0].id,crate::voice::EXTRACTOR]).unwrap();
        let relative: String = db
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1",
                [&mixed.tracks[0].chunks[0].asset_id],
                |r| r.get(0),
            )
            .unwrap();
        db.execute(
            "DELETE FROM app_settings WHERE key='meeting_mix_v1_initialized'",
            [],
        )
        .unwrap();
        initialize(&mut db, s.root()).unwrap();
        initialize(&mut db, s.root()).unwrap();
        assert!(!s.root().join(relative).exists());
        assert!(!partial.exists());
        assert_eq!(db.query_row("SELECT count(*) FROM voice_samples WHERE source_analysis_id IS NULL AND source_track_id IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM audio_tracks WHERE kind='mic'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM analysis_flows", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!db
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
        drop(db);
        assert_eq!(s.voice_people().unwrap()[0].samples.len(), 1);
        assert!(s.recorded_audio_plan(&m).is_ok());
        assert!(s.analysis_flow(&m).unwrap().is_none());
        let output = tempfile::tempdir().unwrap();
        s.create_backup_archive(&output.path().join("reset.monbackup"), &|| false)
            .unwrap();
    }
}
