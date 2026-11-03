//! Durable follow-ups belong to a particular saved transcript, never a scan of
//! old meetings under today's preferences. Job insertion advances intent in the
//! same transaction; a lost response can only read the same operation again.
use super::*;
use crate::{
    diarization::{
        embedding::ExtractorConfig,
        sherpa::{CaptureMode, SherpaConfig},
    },
    local_engine::resident,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Execution {
    pub engine: SherpaConfig,
    pub extractor: ExtractorConfig,
    pub managed: Option<model_profiles::jobs::Snapshot>,
    pub runtime: Option<resident::Snapshot>,
}
pub(crate) struct Prepared {
    preferences: serde_json::Value,
    execution: Result<Execution, String>,
}
pub(crate) fn prepare(storage: &Storage) -> Prepared {
    let result = (|| {
        let preferences = storage.diarization_preferences()?.payload.standard;
        let value = serde_json::to_value(&preferences).map_err(|e| e.to_string())?;
        let execution = (|| {
            let engine = preferences.capture(CaptureMode::Retained, &|| false)?;
            let managed = storage.prepare_model_selection(
                "standard",
                model_profiles::jobs::Execution::Sherpa(&engine),
                &|| false,
            )?;
            let extractor =
                ExtractorConfig::capture(&crate::diarization::known::helper_path()?, &engine)?;
            let runtime = resident::prepare(storage.root(), "sherpa")?;
            Ok(Execution {
                engine,
                extractor,
                managed,
                runtime,
            })
        })();
        Ok::<_, String>((value, execution))
    })();
    match result {
        Ok((preferences, execution)) => Prepared {
            preferences,
            execution,
        },
        Err(error) => Prepared {
            preferences: serde_json::Value::Null,
            execution: Err(error),
        },
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Policy {
    pub schema_version: u32,
    pub completion_enabled: bool,
    pub completion: completion_preferences::CompletionPreferences,
    pub matching: voice_preferences::VoiceMatchingPreferences,
    pub book_hash: String,
    pub execution: Option<Execution>,
}
#[derive(Clone, Debug)]
pub(crate) struct Intent {
    pub id: String,
    pub meeting_id: String,
    pub analysis_id: String,
    pub current_analysis_id: String,
    pub working_hash: String,
    pub phase: String,
    pub completion_operation_id: String,
    pub matching_operation_id: String,
    pub completion_job_id: Option<String>,
    pub matching_job_id: Option<String>,
    pub policy: Policy,
}

pub(super) fn record(
    tx: &rusqlite::Transaction<'_>,
    meeting: &str,
    analysis: &str,
    enabled: bool,
    prepared: &Prepared,
) -> Result<(), String> {
    let current = serde_json::to_value(diarization_preferences::load(tx)?.payload.standard)
        .map_err(|e| e.to_string())?;
    let result = if current != prepared.preferences {
        Err("後続処理の準備中にモデル設定が変更されました。解析結果は保存済みです".to_owned())
    } else {
        prepared.execution.clone()
    };
    record_fixed(tx, meeting, analysis, enabled, result)
}
fn record_fixed(
    tx: &rusqlite::Transaction<'_>,
    meeting: &str,
    analysis: &str,
    enabled: bool,
    result: Result<Execution, String>,
) -> Result<(), String> {
    let (_, _, working_hash) = voice::working(tx, meeting)?;
    let any_voice: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM voice_samples)", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let phase = if !enabled && !any_voice {
        "completed"
    } else if enabled {
        "completion_pending"
    } else {
        "matching_pending"
    };
    let error = result.as_ref().err().cloned();
    let policy = Policy {
        schema_version: 1,
        completion_enabled: enabled,
        completion: completion_preferences::load(tx)?.payload,
        matching: voice_preferences::load(tx)?.payload,
        book_hash: voice::book_hash(tx)?,
        execution: result.ok(),
    };
    let text = serde_json::to_string(&policy).map_err(|e| e.to_string())?;
    if text.len() > 1024 * 1024 {
        return Err("後続処理の固定条件が上限を超えています".into());
    }
    let now = now_ms()?;
    tx.execute("INSERT INTO post_analysis_intents(id,meeting_id,analysis_id,current_analysis_id,working_hash,spec_json,spec_hash,phase,completion_operation_id,matching_operation_id,error_message,created_at,updated_at) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11) ON CONFLICT(analysis_id) DO NOTHING",
        params![Uuid::new_v4().to_string(),meeting,analysis,working_hash,text,format!("{:x}",Sha256::digest(text.as_bytes())),phase,Uuid::new_v4().to_string(),Uuid::new_v4().to_string(),error,now]).map_err(|e|e.to_string())?;
    Ok(())
}
pub(super) fn record_after_completion(
    tx: &rusqlite::Transaction<'_>,
    spec: &JobSpec,
    plan: &crate::speaker_completion::Plan,
    analysis: &str,
) -> Result<(), String> {
    record_fixed(
        tx,
        &spec.meeting_id,
        analysis,
        false,
        Ok(Execution {
            engine: plan.engine.clone(),
            extractor: plan.extractor.clone(),
            managed: model_profiles::jobs::read(&spec.profile_snapshot)?,
            runtime: plan.resident_runtime.clone(),
        }),
    )
}
fn read(db: &Connection, id: &str) -> Result<Intent, String> {
    db.query_row("SELECT id,meeting_id,analysis_id,current_analysis_id,working_hash,phase,completion_operation_id,matching_operation_id,completion_job_id,matching_job_id,spec_json,spec_hash FROM post_analysis_intents WHERE id=?1",[id],|r|{
        let text:String=r.get(10)?;let hash:String=r.get(11)?;
        let decode=|e:String|rusqlite::Error::FromSqlConversionFailure(10,rusqlite::types::Type::Text,Box::new(std::io::Error::other(e)));
        if text.len()>1024*1024 || format!("{:x}",Sha256::digest(text.as_bytes()))!=hash {return Err(decode("後続処理の固定条件が変更されています".into()));}
        let policy:Policy=serde_json::from_str(&text).map_err(|e|decode(e.to_string()))?;
        if policy.schema_version!=1 || !jobs::valid_hash(&policy.book_hash) {return Err(decode("未対応または不正な後続処理形式です".into()));}
        policy.completion.validate().and_then(|_|policy.matching.validate()).map_err(decode)?;
        Ok(Intent{id:r.get(0)?,meeting_id:r.get(1)?,analysis_id:r.get(2)?,current_analysis_id:r.get(3)?,working_hash:r.get(4)?,phase:r.get(5)?,completion_operation_id:r.get(6)?,matching_operation_id:r.get(7)?,completion_job_id:r.get(8)?,matching_job_id:r.get(9)?,policy})
    }).map_err(|e|e.to_string())
}

/// Validate relational ownership as well as JSON integrity. The schema-35
/// portable reader deliberately has no intent table.
pub(in crate::storage) fn validate(db: &Connection) -> Result<(), String> {
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='post_analysis_intents' AND type='table')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {
        return Ok(());
    }
    for sql in [
        "SELECT EXISTS(SELECT 1 FROM post_analysis_intents i JOIN analysis_revisions a ON a.id=i.analysis_id JOIN analysis_revisions c ON c.id=i.current_analysis_id WHERE a.meeting_id!=i.meeting_id OR c.meeting_id!=i.meeting_id OR a.publication_state!='committed' OR c.publication_state!='committed')",
        "SELECT EXISTS(SELECT 1 FROM post_analysis_intents i JOIN processing_jobs j ON j.id=i.completion_job_id WHERE j.meeting_id!=i.meeting_id OR j.kind!='speaker_completion' OR j.operation_id!=i.completion_operation_id)",
        "SELECT EXISTS(SELECT 1 FROM post_analysis_intents i JOIN processing_jobs j ON j.id=i.matching_job_id WHERE j.meeting_id!=i.meeting_id OR j.kind!='voice_matching' OR j.operation_id!=i.matching_operation_id)",
        "SELECT EXISTS(SELECT 1 FROM post_analysis_intents WHERE (phase='completion_enqueued' AND completion_job_id IS NULL) OR (phase='matching_enqueued' AND matching_job_id IS NULL) OR completion_operation_id=matching_operation_id)",
    ] {if db.query_row(sql,[],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {return Err("後続処理の会議・解析版・操作またはジョブの参照が一致しません".into());}}
    let mut q = db
        .prepare("SELECT id FROM post_analysis_intents ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for id in ids {
        read(db, &id.map_err(|e| e.to_string())?)?;
    }
    Ok(())
}
impl Intent {
    pub(crate) fn execution(&self) -> Result<&Execution, String> {
        self.policy
            .execution
            .as_ref()
            .ok_or("後続処理のモデル準備を確認できません。解析結果は保存されています".into())
    }
    pub(crate) fn check(
        &self,
        tx: &rusqlite::Transaction<'_>,
        analysis: &str,
        working: &str,
    ) -> Result<(), String> {
        let current = read(tx, &self.id)?;
        if current.current_analysis_id != analysis
            || current.working_hash != working
            || current.phase != self.phase
            || voice::book_hash(tx)? != self.policy.book_hash
        {
            return Err("保存確定後に会議・人物登録または後続処理が変更されています".into());
        }
        Ok(())
    }
    pub(crate) fn enqueued(
        &self,
        tx: &rusqlite::Transaction<'_>,
        job: &str,
        completion: bool,
    ) -> Result<(), String> {
        let saved = jobs::load_job(tx, job)?.ok_or("後続処理のジョブがありません")?;
        let expected_kind = if completion {
            JobKind::SpeakerCompletion
        } else {
            JobKind::VoiceMatching
        };
        let expected_operation = if completion {
            &self.completion_operation_id
        } else {
            &self.matching_operation_id
        };
        if saved.meeting_id != self.meeting_id
            || saved.kind != expected_kind
            || &saved.operation_id != expected_operation
        {
            return Err("後続処理とジョブの会議・種別・操作が一致しません".into());
        }
        let sql = if completion {
            "UPDATE post_analysis_intents SET phase='completion_enqueued',completion_job_id=?1,updated_at=?2 WHERE id=?3 AND phase='completion_pending'"
        } else {
            "UPDATE post_analysis_intents SET phase='matching_enqueued',matching_job_id=?1,updated_at=?2 WHERE id=?3 AND phase='matching_pending'"
        };
        if tx
            .execute(sql, params![job, now_ms()?, self.id])
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("後続処理の受付が競合しました".into());
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn saved(root: &Path) -> (Storage, String) {
        let (storage, meeting) = crate::storage::analysis_flows::tests::fixture(root);
        storage.ensure_analysis_flow(&meeting).unwrap();
        for _ in 0..2 {
            crate::analysis_flow::advance(&storage, &|| false, &|_| {}).unwrap();
            let job = storage.acquire_local_inference_job().unwrap().unwrap();
            match job.kind {
                JobKind::Transcription => {
                    crate::transcription::process(&storage, &job.id, &|| false, &|_| {}).unwrap();
                }
                JobKind::Diarization => {
                    crate::diarization::process(&storage, &job.id, &|| false, &|_| {}).unwrap();
                }
                _ => panic!("unexpected fixture job"),
            }
        }
        crate::analysis_flow::advance(&storage, &|| false, &|_| {}).unwrap();
        (storage, meeting)
    }
    #[test]
    fn saved_transcript_and_one_frozen_intent_survive_reopen_without_callback() {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        let original = storage.load_current_analysis(&meeting).unwrap().unwrap();
        let id: String = storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get(0),
            )
            .unwrap();
        let first = {
            let db = storage.connection.lock().unwrap();
            read(&db, &id).unwrap()
        };
        let mut changed = first.policy.completion;
        changed.similarity_threshold_milli = 500;
        storage
            .save_speaker_completion_preferences(0, changed)
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            assert!(db
                .execute(
                    "UPDATE post_analysis_intents SET spec_json='{}' WHERE id=?1",
                    [&id]
                )
                .is_err());
        }
        drop(storage);
        let reopened = Storage::open(root.path().to_owned()).unwrap();
        let db = reopened.connection.lock().unwrap();
        let next = read(&db, &id).unwrap();
        assert_eq!(next.analysis_id, original.id);
        assert_eq!(next.policy.completion, first.policy.completion);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn job_registration_and_intent_acknowledgement_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        let mut db = storage.connection.lock().unwrap();
        let id: String = db
            .query_row(
                "SELECT id FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get(0),
            )
            .unwrap();
        db.execute(
            "UPDATE post_analysis_intents SET phase='completion_pending' WHERE id=?1",
            [&id],
        )
        .unwrap();
        let intent = read(&db, &id).unwrap();
        let hash: String = db
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id=?1",
                [&intent.current_analysis_id],
                |r| r.get(0),
            )
            .unwrap();
        let spec = JobSpec {
            meeting_id: meeting,
            input_analysis_id: Some(intent.current_analysis_id.clone()),
            kind: JobKind::SpeakerCompletion,
            operation_id: intent.completion_operation_id.clone(),
            input_hash: hash,
            profile_snapshot: serde_json::json!({}),
            parameters: serde_json::json!({"completion":{"schemaVersion":crate::current_formats::COMPLETION}}),
            total_units: Some(1),
        };
        {
            let tx = db.transaction().unwrap();
            let job = jobs::enqueue_in_transaction(&tx, &spec).unwrap();
            intent.enqueued(&tx, &job.id, true).unwrap();
        }
        assert_eq!(read(&db, &id).unwrap().phase, "completion_pending");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM processing_jobs WHERE operation_id=?1",
                [&intent.completion_operation_id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
        {
            let tx = db.transaction().unwrap();
            let job = jobs::enqueue_in_transaction(&tx, &spec).unwrap();
            intent.enqueued(&tx, &job.id, true).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(read(&db, &id).unwrap().phase, "completion_enqueued");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM processing_jobs WHERE operation_id=?1",
                [&intent.completion_operation_id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn followup_backup_keeps_current_transcript_without_job_or_intent_history() {
        let root = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        let source_analysis = storage.load_current_analysis(&meeting).unwrap().unwrap().id;
        let path = output.path().join("followups.monbackup");
        storage.create_backup_archive(&path, &|| false).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = Storage::open(target.path().into()).unwrap();
        let prepared = restored
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        prepared
            .publish(
                &restored,
                &prepared.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap();
        let db = restored.connection.lock().unwrap();
        validate(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM post_analysis_intents", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        let restored_meeting = restored.list_meetings().unwrap().remove(0).id;
        assert_ne!(restored_meeting, meeting);
        assert_ne!(
            restored
                .load_current_analysis(&restored_meeting)
                .unwrap()
                .unwrap()
                .id,
            source_analysis
        );
        restored.drain_post_analysis(&|| false).unwrap();
        assert!(restored.list_jobs(&restored_meeting).unwrap().is_empty());
    }
    #[test]
    fn drain_is_idempotent_after_restart_and_cancellation_preserves_pending_work() {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE post_analysis_intents SET phase='matching_pending' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        }
        storage.drain_post_analysis(&|| true).unwrap();
        {
            let db = storage.connection.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                    [&meeting],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "matching_pending"
            );
        }
        drop(storage);
        let reopened = Storage::open(root.path().into()).unwrap();
        reopened.drain_post_analysis(&|| false).unwrap();
        reopened.drain_post_analysis(&|| false).unwrap();
        let db = reopened.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "completed"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn reversible_meeting_reservation_pauses_without_poisoning_the_intent() {
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE post_analysis_intents SET phase='matching_pending' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        }
        storage.reserve_meeting_deletion(&meeting).unwrap();
        storage.drain_post_analysis(&|| false).unwrap();
        {
            let db = storage.connection.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                    [&meeting],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "matching_pending"
            );
        }
        storage.release_meeting_deletion(&meeting).unwrap();
        storage.drain_post_analysis(&|| false).unwrap();
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "completed"
        );
    }
    #[test]
    fn concurrent_publication_and_worker_drains_do_not_poison_or_duplicate_an_intent() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        };
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE post_analysis_intents SET phase='matching_pending' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        }
        let storage = Arc::new(storage);
        let entered = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let child = {
            let storage = storage.clone();
            let entered = entered.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                let first = AtomicBool::new(true);
                storage.drain_post_analysis(&|| {
                    if first.swap(false, Ordering::SeqCst) {
                        entered.wait();
                        resume.wait();
                    }
                    false
                })
            })
        };
        entered.wait();
        storage.drain_post_analysis(&|| true).unwrap();
        let waiting = Arc::new(Barrier::new(2));
        let follower = {
            let storage = storage.clone();
            let waiting = waiting.clone();
            std::thread::spawn(move || {
                let first = AtomicBool::new(true);
                storage.drain_post_analysis(&|| {
                    if first.swap(false, Ordering::SeqCst) {
                        waiting.wait();
                    }
                    false
                })
            })
        };
        waiting.wait();
        {
            let db = storage.connection.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                    [&meeting],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "matching_pending"
            );
        }
        resume.wait();
        child.join().unwrap().unwrap();
        follower.join().unwrap().unwrap();
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT phase FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "completed"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM post_analysis_intents WHERE meeting_id=?1",
                [&meeting],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn eligible_completion_uses_its_own_hash_domain_and_the_intent_voice_guard() {
        let (root, storage, meeting, _, mut plan) =
            crate::storage::speaker_completion::tests::fixture();
        let proof = |name: &str| {
            let path = root.path().join(name);
            std::fs::write(&path, name).unwrap();
            crate::local_engine::Fingerprint::capture_retained(&path, &|| false).unwrap()
        };
        plan.engine.executable = proof("sherpa-cli");
        plan.engine.segmentation_model = proof("seg.onnx");
        plan.engine.embedding_model = proof("emb.onnx");
        plan.engine.runtime_libraries = vec![proof("runtime.dylib")];
        plan.extractor.library = plan.engine.runtime_libraries[0].clone();
        plan.extractor.helper = proof("extractor");
        let intent = {
            let mut db = storage.connection.lock().unwrap();
            let tx = db.transaction().unwrap();
            let analysis = voice::working(&tx, &meeting).unwrap().0.id;
            record_fixed(
                &tx,
                &meeting,
                &analysis,
                true,
                Ok(Execution {
                    engine: plan.engine,
                    extractor: plan.extractor,
                    managed: None,
                    runtime: plan.resident_runtime,
                }),
            )
            .unwrap();
            let id: String = tx
                .query_row(
                    "SELECT id FROM post_analysis_intents WHERE meeting_id=?1",
                    [&meeting],
                    |r| r.get(0),
                )
                .unwrap();
            let intent = read(&tx, &id).unwrap();
            tx.commit().unwrap();
            intent
        };
        let first = storage.enqueue_completion_intent(&intent).unwrap().unwrap();
        assert_eq!(first.operation_id, intent.completion_operation_id);
        assert_eq!(first.kind, JobKind::SpeakerCompletion);
        let current = {
            let db = storage.connection.lock().unwrap();
            read(&db, &intent.id).unwrap()
        };
        assert_eq!(current.phase, "completion_enqueued");
        storage.drain_post_analysis(&|| false).unwrap();
        assert_eq!(
            storage
                .list_jobs(&meeting)
                .unwrap()
                .iter()
                .filter(|job| job.kind == JobKind::SpeakerCompletion)
                .count(),
            1
        );
    }
    #[test]
    fn unmanaged_execution_conditions_are_excluded_from_current_backup() {
        let (_root, storage, meeting, spec, plan) =
            crate::storage::speaker_completion::tests::fixture();
        {
            let mut db = storage.connection.lock().unwrap();
            let tx = db.transaction().unwrap();
            record_fixed(
                &tx,
                &meeting,
                spec.input_analysis_id.as_deref().unwrap(),
                true,
                Ok(Execution {
                    engine: plan.engine,
                    extractor: plan.extractor,
                    managed: None,
                    runtime: plan.resident_runtime,
                }),
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("unmanaged.monbackup");
        storage.create_backup_archive(&path, &|| false).unwrap();
        let target = tempfile::tempdir().unwrap();
        let restored = Storage::open(target.path().into()).unwrap();
        let prepared = restored
            .prepare_additive_restore(&path, &|| false, &|_| {})
            .unwrap();
        prepared
            .publish(
                &restored,
                &prepared.preview().confirmation_id,
                &|| false,
                &|_| {},
            )
            .unwrap();
        let db = restored.connection.lock().unwrap();
        validate(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM post_analysis_intents", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn concurrently_deleted_intent_is_skipped_without_poisoning_the_drain() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = tempfile::tempdir().unwrap();
        let (storage, meeting) = saved(root.path());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE post_analysis_intents SET phase='matching_pending' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        }
        let first = AtomicBool::new(true);
        storage
            .drain_post_analysis(&|| {
                if first.swap(false, Ordering::SeqCst) {
                    storage
                        .connection
                        .lock()
                        .unwrap()
                        .execute(
                            "DELETE FROM post_analysis_intents WHERE meeting_id=?1",
                            [&meeting],
                        )
                        .unwrap();
                }
                false
            })
            .unwrap();
        assert!(storage.load_current_analysis(&meeting).unwrap().is_some());
    }
    #[test]
    fn completion_publication_commits_the_next_intent_before_any_callback_or_restart() {
        let (root, storage, meeting, mut spec, plan) =
            crate::storage::speaker_completion::tests::fixture();
        let intent = {
            let mut db = storage.connection.lock().unwrap();
            let tx = db.transaction().unwrap();
            record_fixed(
                &tx,
                &meeting,
                spec.input_analysis_id.as_deref().unwrap(),
                true,
                Ok(Execution {
                    engine: plan.engine.clone(),
                    extractor: plan.extractor.clone(),
                    managed: None,
                    runtime: plan.resident_runtime.clone(),
                }),
            )
            .unwrap();
            let id: String = tx
                .query_row(
                    "SELECT id FROM post_analysis_intents WHERE meeting_id=?1",
                    [&meeting],
                    |r| r.get(0),
                )
                .unwrap();
            let intent = read(&tx, &id).unwrap();
            spec.operation_id = intent.completion_operation_id.clone();
            let job = jobs::enqueue_in_transaction(&tx, &spec).unwrap();
            intent.enqueued(&tx, &job.id, true).unwrap();
            tx.commit().unwrap();
            intent
        };
        let job = storage.acquire_local_inference_job().unwrap().unwrap();
        let old_snapshot = {
            let db = storage.connection.lock().unwrap();
            read(&db, &intent.id).unwrap()
        };
        let decision = crate::speaker_completion::Decision {
            needs_review: false,
            lineage: plan.targets[0].lineage.clone(),
            speaker_id: Some(plan.references[0].speaker_id.clone()),
            body_best: Some(plan.references[0].speaker_id.clone()),
            similarity: Some(0.95),
            runner_up: Some(0.1),
            reason: "applied".into(),
        };
        storage
            .publish_completion_result(&spec, &plan, &[decision], &job.id)
            .unwrap();
        // Simulate a drain that read completion_enqueued just before the
        // publisher committed the matching handoff in another thread.
        storage.advance_followup(&old_snapshot).unwrap();
        {
            let db = storage.connection.lock().unwrap();
            assert_eq!(read(&db, &intent.id).unwrap().phase, "matching_pending");
        }
        let output = storage.load_current_analysis(&meeting).unwrap().unwrap().id;
        assert_ne!(output, intent.current_analysis_id);
        drop(storage);
        let reopened = Storage::open(root.path().into()).unwrap();
        {
            let db = reopened.connection.lock().unwrap();
            let next = read(&db, &intent.id).unwrap();
            assert_eq!(next.phase, "matching_pending");
            assert_eq!(next.current_analysis_id, output);
            validate(&db).unwrap();
        }
        reopened.drain_post_analysis(&|| false).unwrap();
        let db = reopened.connection.lock().unwrap();
        assert_eq!(read(&db, &intent.id).unwrap().phase, "completed");
    }
}
impl Storage {
    fn followup_phase(
        &self,
        intent: &Intent,
        phase: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let id = &intent.id;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // Another producer may atomically publish completion while this drain
        // is inspecting its old job snapshot. Never overwrite that handoff.
        let changed=db.execute(
            "UPDATE post_analysis_intents SET phase=?1,error_message=?2,updated_at=?3 WHERE id=?4 AND phase=?5 AND current_analysis_id=?6 AND working_hash=?7",
            params![phase, error, now_ms()?, id,intent.phase,intent.current_analysis_id,intent.working_hash],
        )
        .map_err(|e| e.to_string())?;
        if phase == "blocked" && changed == 1 {
            db.execute("UPDATE analysis_flows SET error_message=?1 WHERE stage='review' AND meeting_id=(SELECT meeting_id FROM post_analysis_intents WHERE id=?2)",params![error,id]).map_err(|e|e.to_string())?;
        }
        Ok(())
    }
    pub(crate) fn drain_post_analysis(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        // Publication callbacks and the worker can both request a drain. A
        // successful phase change must not become another caller's permanent
        // failure. Wait so this caller also sees intents saved after the first
        // drain took its list; cancellation leaves their durable rows pending.
        let _drain = loop {
            match self.post_analysis_drain.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    if cancelled() {
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            }
        };
        self.require_data_writable()?;
        let ids = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut q=db.prepare("SELECT id FROM post_analysis_intents WHERE phase IN ('completion_pending','completion_enqueued','matching_pending','matching_enqueued') ORDER BY created_at,rowid LIMIT 4097").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            let v = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if v.len() > 4096 {
                return Err("後続処理の確認件数が上限を超えています".into());
            }
            v
        };
        for id in ids {
            for _ in 0..4 {
                if cancelled() {
                    return Ok(());
                }
                let (intent, _meeting_work) = {
                    let db = self
                        .connection
                        .lock()
                        .map_err(|_| "DBロックを取得できません")?;
                    let exists: bool = db
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM post_analysis_intents WHERE id=?1)",
                            [&id],
                            |row| row.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    if !exists {
                        break;
                    } // A concurrent meeting deletion already removed it.
                    let intent = read(&db, &id)?;
                    // Check and lease while holding the DB lock used to reserve
                    // meetings, so a reversible review cannot race with writes.
                    if super::meeting_access::reserved(&db, &intent.meeting_id)? {
                        break;
                    }
                    let owner = self.begin_meeting_work_locked(
                        &db,
                        &intent.meeting_id,
                        "post_analysis",
                        None,
                    )?;
                    (intent, owner)
                };
                if !matches!(
                    intent.phase.as_str(),
                    "completion_pending"
                        | "completion_enqueued"
                        | "matching_pending"
                        | "matching_enqueued"
                ) {
                    break;
                }
                if let Err(error) = self.advance_followup(&intent) {
                    self.followup_phase(&intent, "blocked", Some(&error))?;
                    crate::internal_logs::report_error("post_analysis", &error);
                    break;
                }
                let next = {
                    let db = self
                        .connection
                        .lock()
                        .map_err(|_| "DBロックを取得できません")?;
                    read(&db, &id)?
                };
                if next.phase == intent.phase {
                    break;
                }
            }
        }
        Ok(())
    }
    fn advance_followup(&self, intent: &Intent) -> Result<(), String> {
        if matches!(
            intent.phase.as_str(),
            "completion_enqueued" | "matching_enqueued"
        ) {
            let completion = intent.phase == "completion_enqueued";
            let job = self.get_job(
                if completion {
                    intent.completion_job_id.as_deref()
                } else {
                    intent.matching_job_id.as_deref()
                }
                .ok_or("後続ジョブの参照がありません")?,
            )?;
            if matches!(job.state.as_str(), "failed" | "cancelled") {
                return Err(
                    "後続処理が完了していません。通常の再生成で対象を再確認してください".into(),
                );
            }
            if job.state != "succeeded" {
                return Ok(());
            }
            if !completion {
                return self.followup_phase(intent, "completed", None);
            }
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let (view, _, hash) = voice::working(&tx, &intent.meeting_id)?;
            let expected = job
                .output_analysis_id
                .as_deref()
                .unwrap_or(&intent.current_analysis_id);
            if view.id != expected
                || (job.output_analysis_id.is_none() && hash != intent.working_hash)
            {
                drop(tx);
                drop(db);
                return self.followup_phase(
                    intent,
                    "stale",
                    Some("補完後に会議が変更されています"),
                );
            }
            tx.execute("UPDATE post_analysis_intents SET phase='matching_pending',current_analysis_id=?1,working_hash=?2,updated_at=?3 WHERE id=?4 AND phase='completion_enqueued'",params![view.id,hash,now_ms()?,intent.id]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(());
        }
        {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let (view, _, hash) = voice::working(&tx, &intent.meeting_id)?;
            if intent.check(&tx, &view.id, &hash).is_err() {
                drop(tx);
                drop(db);
                return self.followup_phase(
                    intent,
                    "stale",
                    Some("保存確定後に会議または人物登録が変更されています"),
                );
            }
        }
        if intent.phase == "completion_pending" {
            if self.enqueue_completion_intent(intent)?.is_none() {
                self.followup_phase(intent, "matching_pending", None)?;
            }
        } else if intent.phase == "matching_pending" {
            let context = self.voice_context(&intent.meeting_id)?;
            if context.windows.is_empty() || context.people.iter().all(|p| p.samples.is_empty()) {
                return self.followup_phase(intent, "completed", None);
            }
            self.start_automatic_voice_intent(
                voice::Start {
                    meeting_id: intent.meeting_id.clone(),
                    operation_id: intent.matching_operation_id.clone(),
                    analysis_id: intent.current_analysis_id.clone(),
                    working_hash: intent.working_hash.clone(),
                    book_hash: intent.policy.book_hash.clone(),
                    speaker_id: None,
                    person_id: None,
                    person_name: None,
                    selected_windows: None,
                },
                intent,
            )?;
        }
        Ok(())
    }
}
