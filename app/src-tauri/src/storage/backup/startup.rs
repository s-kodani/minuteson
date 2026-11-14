//! Startup-only reconstruction from a private copy of a rejected database.
//! User content uses the typed backup constructor; operational journals and
//! native ownership receipts are never copied into the new database.
use super::*;
use crate::model_inspection::Source;
use std::collections::BTreeMap;

pub(crate) struct Prepared {
    stage: scratch::Stage,
    pub(crate) root: PathBuf,
    pub(crate) models: PathBuf,
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(crate) fn prepare(
    database_copy: &Path,
    source_root: &Path,
    models_root: &Path,
    workspace: &Path,
    model_workspace: &Path,
    progress: &impl Fn(&str),
) -> Result<Prepared, String> {
    let stage = scratch::Stage::create(workspace)?;
    scratch::directories(&stage.path().join("assets"))?;
    let root = stage.path().join("reconstructed");
    scratch::directories(&root)?;
    let models = model_workspace.join("new-models");
    crate::native_fs::private_directories(&models, false)?;
    // SQLite can recover WAL/hot journals only in this private copy. The
    // original database and its sidecars have not been opened or changed.
    let mut source = Connection::open(database_copy).map_err(err)?;
    source
        .execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")
        .map_err(err)?;
    verify_database_schema(&source).map_err(err)?;
    let integrity: String = source
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(err)?;
    if integrity != "ok" {
        return Err("会議DBをすべて回収できません。初期化せず元データを保持します".into());
    }
    // Unpublished destructive/storage operations and incomplete source audio
    // need resolution first. Transient analysis/export queues are rebuilt;
    // they do not prevent collection of the current committed user content.
    let unresolved: bool = source.query_row(
        "SELECT EXISTS(SELECT 1 FROM restore_batches WHERE state='prepared') OR EXISTS(SELECT 1 FROM recording_sessions WHERE state!='saved') OR EXISTS(SELECT 1 FROM audio_imports WHERE state!='ready')", [], |r| r.get(0),
    ).map_err(err)?;
    if unresolved || super::super::deletion::fence::has_pending(&source)? {
        return Err("未確定の保存操作があるため、全データの回収を確認できません。初期化せず元データを保持します".into());
    }
    progress("会議・文字起こし・話者分離・議事録・ボイス登録を回収しています");
    let (contents, mut assets) = contents::Contents::capture(
        &mut source,
        stage.path(),
        &deleted_audio::Native::empty(),
        &|| false,
    )?;
    let voices = super::super::voice::audio::assets(&source)?;
    for (id, meeting, missing) in contents.assets() {
        if missing {
            return Err("回収できない会議音声があります。元データを保持します".into());
        }
        let asset = if meeting.is_empty() {
            voices
                .iter()
                .find(|asset| asset.id == id)
                .cloned()
                .ok_or("登録音声の情報がありません")?
        } else {
            source.query_row("SELECT id,meeting_id,relative_path,media_type,size_bytes,content_hash,state FROM file_assets WHERE id=?1 AND meeting_id=?2",params![id,meeting],|r| Ok(FileAsset{id:r.get(0)?,meeting_id:r.get(1)?,relative_path:r.get(2)?,media_type:r.get(3)?,size_bytes:r.get(4)?,content_hash:r.get(5)?,state:r.get(6)?})).map_err(err)?
        };
        if asset.state != "ready" {
            return Err("保存が完了していない音声があります。元データを保持します".into());
        }
        if meeting.is_empty() {
            let person: String = source
                .query_row(
                    "SELECT person_id FROM voice_samples WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if super::super::voice::audio::relative(&person, &id)? != asset.relative_path {
                return Err("登録音声の所属を確認できません。元データを保持します".into());
            }
        } else if !asset
            .relative_path
            .starts_with(&format!("meetings/{meeting}/"))
        {
            return Err("会議音声の所属を確認できません。元データを保持します".into());
        }
        let key = Uuid::new_v4().to_string();
        let path = stage.path().join("assets").join(&key);
        let mut input = Source::open(source_root, &asset.relative_path)?;
        let size = u64::try_from(asset.size_bytes).map_err(err)?;
        check_capacity(stage.path(), size)?;
        let mut output = private_file(&path)?;
        let (actual_size, hash) = stream(&mut input.file, Some(&mut output), size, &|| false)?;
        if actual_size != size || hash != asset.content_hash {
            return Err("音声の容量・SHAが一致しません。元データを保持します".into());
        }
        output.sync_all().map_err(err)?;
        input.verify()?;
        assets.push(StagedAsset { asset, key, path });
    }
    let paths: BTreeMap<_, _> = assets
        .iter()
        .map(|asset| (asset.asset.id.clone(), asset.path.clone()))
        .collect();
    contents.verify_audio(&paths, &|| false)?;
    let mut destinations = BTreeMap::new();
    for asset in &assets {
        let id = &asset.asset.id;
        if !crate::model_compatibility::canonical_id(id) {
            return Err("回収音声の識別子が不正です".into());
        }
        let relative = if asset.asset.meeting_id.is_empty() {
            let person: String = source
                .query_row(
                    "SELECT person_id FROM voice_samples WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            super::super::voice::audio::relative(&person, id)?
        } else {
            let meeting = &asset.asset.meeting_id;
            if !crate::model_compatibility::canonical_id(meeting) {
                return Err("回収会議の識別子が不正です".into());
            }
            format!(
                "meetings/{meeting}/{id}.{}",
                if asset.asset.media_type == "application/json" {
                    "json"
                } else {
                    "wav"
                }
            )
        };
        let target = root.join(&relative);
        scratch::directories(target.parent().ok_or("回収音声の保存先がありません")?)?;
        std::fs::rename(&asset.path, &target).map_err(err)?;
        destinations.insert((asset.asset.meeting_id.clone(), id.clone()), relative);
    }
    progress("モデル本体と設定を回収しています");
    let ids = source
        .prepare("SELECT id FROM model_packages ORDER BY id")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let target_models = crate::model_placement::fs::Directory::root(&models)?;
    if models_root.exists() {
        for entry in std::fs::read_dir(models_root).map_err(err)? {
            let entry = entry.map_err(err)?;
            if !ids.iter().any(|id| entry.file_name() == id.as_str()) {
                return Err(
                    "登録を確認できないモデルがあります。初期化せず元データを保持します".into(),
                );
            }
        }
    }
    for id in ids {
        let (record, placed) = super::super::model_placements::load(&source, &id)?
            .ok_or("モデルの登録情報がありません")?;
        if !placed {
            return Err("回収できないモデルがあります。初期化せず元データを保持します".into());
        }
        let package = models_root.join(&id);
        let report = crate::model_inspection::inspect(&package, &|| false)?;
        if report.review_hash != record.review_hash
            || report.manifest != record.manifest
            || report.files != record.files
        {
            return Err("登録モデルと本体が一致しません。元データを保持します".into());
        }
        check_capacity(&models, report.total_bytes)?;
        let target = target_models.child(&id, true)?;
        crate::model_placement::copy_package(&package, &report, &target, None, &|| false)?;
        let copied = crate::model_inspection::inspect(target.path(), &|| false)?;
        if copied.review_hash != record.review_hash {
            return Err("モデルの回収内容を確認できません。元データを保持します".into());
        }
    }
    let mut db = contents.write_database(
        &root.join("meetings.sqlite3"),
        &|_, _, id| Ok(id.to_owned()),
        &|meeting, id| {
            destinations
                .get(&(meeting.into(), id.into()))
                .cloned()
                .ok_or_else(|| "回収音声の対応がありません".into())
        },
        &|| false,
    )?;
    let tx = db.transaction().map_err(err)?;
    tx.execute_batch("PRAGMA defer_foreign_keys=ON;")
        .map_err(err)?;
    for table in [
        "model_packages",
        "model_files",
        "model_compatibility_checks",
    ] {
        copy_table(&source, &tx, table)?;
    }
    copy_model_profiles(&source, &tx)?;
    for table in ["voice_policies", "app_settings"] {
        copy_table(&source, &tx, table)?;
    }
    tx.commit().map_err(err)?;
    validate_settings(&db)?;
    validate_database(&mut db, &|| false)?;
    drop(db);
    // Use ordinary readers on the reconstructed copy before resetting anything.
    // This also checks typed preferences and model/profile bindings.
    progress("回収したデータの再登録と整合性を検証しています");
    let storage = Storage::open_with_models(root.clone(), models_root.into()).map_err(err)?;
    storage.analysis_preferences()?;
    storage.diarization_preferences()?;
    storage.minutes_preferences()?;
    storage.ui_preferences()?;
    storage.voice_matching_preferences()?;
    storage.speaker_completion_preferences()?;
    storage.minutes_prompt_preferences()?;
    storage.model_download_guidance()?;
    storage.list_model_placements()?;
    drop(storage);
    crate::native_fs::sync_path(&root)?;
    crate::native_fs::sync_path(&models)?;
    Ok(Prepared {
        stage,
        root,
        models,
    })
}

#[cfg(test)]
pub(crate) fn populate_fixture(storage: &Storage) -> (String, String, String) {
    use crate::model_inspection::{Engine, PackageFile, PackageManifest, Role, Target};
    let meeting = archive::tests::meeting(storage, "startup-recovery");
    let person = Uuid::new_v4().to_string();
    let sample = Uuid::new_v4().to_string();
    let relative = super::super::voice::audio::relative(&person, &sample).unwrap();
    let mut audio = crate::audio::wave::header(160).unwrap();
    audio.resize(audio.len() + 320, 0);
    crate::native_fs::private_directories(&storage.root().join("voices"), false).unwrap();
    crate::native_fs::private_directories(storage.root().join(&relative).parent().unwrap(), false)
        .unwrap();
    std::fs::write(storage.root().join(&relative), &audio).unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute(
            "INSERT INTO voice_people VALUES(?1,'登録した声',1,0)",
            [&person],
        )
        .unwrap();
        db.execute("INSERT INTO voice_samples(id,person_id,model_hash,extractor_version,dimension,vector,start_us,end_us,created_at) VALUES(?1,?2,?3,?4,2,?5,0,10000,0)", params![sample,person,"a".repeat(64),crate::voice::EXTRACTOR,crate::voice::encode(&[1.,0.]).unwrap()]).unwrap();
        db.execute("INSERT INTO voice_sample_audio(id,relative_path,size_bytes,content_hash,frame_count,state) VALUES(?1,?2,?3,?4,160,'ready')", params![sample,relative,audio.len() as i64,format!("{:x}",Sha256::digest(&audio))]).unwrap();
        let json = r#"{"schemaVersion":1,"theme":"dark","colorTheme":"purple"}"#;
        db.execute(
            "INSERT INTO app_settings VALUES('ui_preferences',?1,?2,1,0)",
            params![json, format!("{:x}", Sha256::digest(json.as_bytes()))],
        )
        .unwrap();
        let markdown = "# 回収する議事録\n決定事項を保持します。";
        db.execute(
            "INSERT INTO minutes_markdown(meeting_id,markdown,content_hash,lock_version,operation_id,base_lock_version,updated_at) VALUES(?1,?2,?3,1,?4,0,0)",
            params![
                meeting,
                markdown,
                super::super::minutes_markdown::content_hash(markdown),
                Uuid::new_v4().to_string()
            ],
        )
        .unwrap();
    }
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("engine-cli"), b"fixture; never execute").unwrap();
    std::fs::write(source.path().join("model.bin"), b"recovered model bytes").unwrap();
    let manifest = PackageManifest {
        backend: None,
        schema_version: 1,
        engine: Engine::Whisper,
        engine_version: "fixture".into(),
        model_name: "回収するモデル".into(),
        model_version: "1".into(),
        quantization: None,
        license_ref: "fixture".into(),
        target: Target::current().unwrap(),
        files: vec![
            PackageFile {
                relative_path: "engine-cli".into(),
                role: Role::Executable,
                expected_size: None,
                expected_hash: None,
            },
            PackageFile {
                relative_path: "model.bin".into(),
                role: Role::Asr,
                expected_size: None,
                expected_hash: None,
            },
        ],
    };
    std::fs::write(
        source.path().join(crate::model_inspection::MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let report = crate::model_inspection::inspect(source.path(), &|| false).unwrap();
    let model = Uuid::new_v4().to_string();
    crate::native_fs::private_directories(storage.native_models_root(), false).unwrap();
    crate::model_placement::place(
        storage,
        storage.native_models_root(),
        source.path(),
        &model,
        &report.review_hash,
        &|| false,
    )
    .unwrap();
    (meeting, person, model)
}

fn validate_settings(db: &Connection) -> Result<(), String> {
    let mut query = db
        .prepare("SELECT value_json,content_hash FROM app_settings")
        .map_err(err)?;
    let mut rows = query.query([]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let json: String = row.get(0).map_err(err)?;
        let hash: String = row.get(1).map_err(err)?;
        if json.len() > 1_048_576 || format!("{:x}", Sha256::digest(json.as_bytes())) != hash {
            return Err("設定値を回収できません。初期化せず元データを保持します".into());
        }
    }
    Ok(())
}

fn copy_model_profiles(source: &Connection, target: &Connection) -> Result<(), String> {
    let mut query = source
        .prepare("SELECT id,name,profile_version,state FROM model_profiles ORDER BY rowid")
        .map_err(err)?;
    let mut rows = query.query([]).map_err(err)?;
    let mut completed = Vec::new();
    while let Some(row) = rows.next().map_err(err)? {
        if completed.len() >= 200_000 {
            return Err("回収するモデル構成が上限を超えています".into());
        }
        let id: String = row.get(0).map_err(err)?;
        let name: String = row.get(1).map_err(err)?;
        let version: i64 = row.get(2).map_err(err)?;
        let state: String = row.get(3).map_err(err)?;
        // Components can only be inserted while their profile is preparing.
        // Reapply the original terminal state after all components exist,
        // following the same immutable-profile transitions as ordinary setup.
        let staging_state = if matches!(state.as_str(), "ready" | "unavailable") {
            "preparing"
        } else {
            &state
        };
        target
            .execute(
                "INSERT INTO model_profiles(id,name,profile_version,state) VALUES(?1,?2,?3,?4)",
                params![id, name, version, staging_state],
            )
            .map_err(err)?;
        completed.push((id, state));
    }
    copy_table(source, target, "profile_components")?;
    for (id, state) in completed {
        if matches!(state.as_str(), "ready" | "unavailable") {
            target
                .execute(
                    "UPDATE model_profiles SET state=?1 WHERE id=?2 AND state='preparing'",
                    params![state, id],
                )
                .map_err(err)?;
        }
    }
    Ok(())
}

fn copy_table(source: &Connection, target: &Connection, table: &str) -> Result<(), String> {
    // All identifiers come from the fixed inventory above and trusted schema.
    let mut query = source
        .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
        .map_err(err)?;
    let columns = query.column_count();
    let placeholders = (0..columns).map(|_| "?").collect::<Vec<_>>().join(",");
    target
        .execute(&format!("DELETE FROM \"{table}\""), [])
        .map_err(err)?;
    let mut insert = target
        .prepare(&format!("INSERT INTO \"{table}\" VALUES({placeholders})"))
        .map_err(err)?;
    let mut rows = query.query([]).map_err(err)?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(err)? {
        count += 1;
        if count > 200_000 {
            return Err("回収する登録情報が上限を超えています".into());
        }
        let values = (0..columns)
            .map(|index| row.get::<_, rusqlite::types::Value>(index))
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        insert
            .execute(rusqlite::params_from_iter(values))
            .map_err(err)?;
    }
    Ok(())
}

impl Prepared {
    pub(crate) fn install(&self, storage: &Storage) -> Result<(), String> {
        let source = read_only(&self.root.join("meetings.sqlite3"))?;
        let mut target = storage
            .connection
            .lock()
            .map_err(|_| "復元先DBを取得できません")?;
        Backup::new(&source, &mut target)
            .map_err(err)?
            .run_to_completion(128, std::time::Duration::from_millis(0), None)
            .map_err(err)?;
        validate_database(&mut target, &|| false)?;
        // Keep the scratch registration alive until all staged files are installed.
        let _ = self.stage.path();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_reconstruction_preserves_components_and_sealed_states() {
        let source = Connection::open_in_memory().unwrap();
        let target = Connection::open_in_memory().unwrap();
        for db in [&source, &target] {
            db.execute_batch(crate::storage::SCHEMA_SQL).unwrap();
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        }
        source.execute("INSERT INTO model_packages(id,engine,model_name,model_version,source_type,manifest_json,manifest_hash,state) VALUES('model','whisper_cpp','fixture','1','manual_import','{}',?1,'unavailable')", ["a".repeat(64)]).unwrap();
        for (version, state) in ["preparing", "ready", "unavailable", "incompatible"]
            .into_iter()
            .enumerate()
        {
            let id = Uuid::new_v4().to_string();
            source.execute(
                "INSERT INTO model_profiles(id,name,profile_version,state) VALUES(?1,'standard',?2,?3)",
                params![id, version as i64 + 1, if state == "incompatible" { state } else { "preparing" }],
            ).unwrap();
            if state != "incompatible" {
                source.execute("INSERT INTO profile_components(profile_id,role,package_id,settings_json) VALUES(?1,'asr','model','{}')", [&id]).unwrap();
            }
            if matches!(state, "ready" | "unavailable") {
                source
                    .execute(
                        "UPDATE model_profiles SET state=?1 WHERE id=?2",
                        params![state, id],
                    )
                    .unwrap();
            }
        }
        let snapshot = |db: &Connection| -> String {
            db.query_row("SELECT json_array((SELECT json_group_array(json_array(id,name,profile_version,state)) FROM model_profiles),(SELECT json_group_array(json_array(profile_id,role,package_id,settings_json)) FROM profile_components))", [], |row| row.get(0)).unwrap()
        };
        let before = snapshot(&source);
        copy_table(&source, &target, "model_packages").unwrap();
        copy_model_profiles(&source, &target).unwrap();
        assert_eq!(snapshot(&target), before);
        assert_eq!(snapshot(&source), before);
        assert!(target.execute("INSERT INTO profile_components SELECT profile_id,'extra',package_id,settings_json FROM profile_components WHERE profile_id IN (SELECT id FROM model_profiles WHERE state='ready')", []).is_err());
    }
}
