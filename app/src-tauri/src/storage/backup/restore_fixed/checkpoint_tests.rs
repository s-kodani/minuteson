use super::super::tests::{fixture, project, read, rebuild_analyses, register_assets_except};
use super::*;

fn artifact(
    storage: &Storage,
    meeting: &str,
    job: &str,
    name: &str,
    media: &str,
    bytes: &[u8],
) -> FileAsset {
    let directory = storage.analysis_unit_directory(meeting, job, 0).unwrap();
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    storage
        .register_analysis_artifact(meeting, &path, media)
        .unwrap()
}

fn retry(storage: &Storage, meeting: &str, kind: &str) -> JobSnapshot {
    let id:String=storage.connection.lock().unwrap().query_row("SELECT id FROM processing_jobs WHERE meeting_id=?1 AND kind=?2 AND state='cancelled' ORDER BY attempt DESC LIMIT 1",params![meeting,kind],|r|r.get(0)).unwrap();
    let job = storage.get_job(&id).unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&id).unwrap();
    storage
        .enqueue_retry_job(
            &job,
            &spec,
            checkpoint.as_ref(),
            &Uuid::new_v4().to_string(),
        )
        .unwrap()
}

#[test]
fn native_checkpoints_remap_historical_results_and_keep_omitted_pcm_and_logs_explicit() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "native-results");
    let transcription = retry(&storage, &meeting, "transcription");
    storage
        .acquire_job_kind(Some(JobKind::Transcription))
        .unwrap()
        .unwrap();
    storage
        .save_job_checkpoint(&transcription.id, 0, 0, json!({"outputs":[]}))
        .unwrap();
    let wav = artifact(
        &storage,
        &meeting,
        &transcription.id,
        "input.wav",
        "audio/wav",
        b"synthetic intermediate PCM fixture",
    );
    let recognized = "自然文に含まれる元IDは置換しない";
    let json=artifact(&storage,&meeting,&transcription.id,"result.json","application/json",&serde_json::to_vec(&json!({"model":{"multilingual":true},"params":{"model":"/Users/source-machine/models/model.bin","language":"ja","translate":false},"transcription":[{"offsets":{"from":0,"to":1000},"text":recognized}]})).unwrap());
    storage.save_job_checkpoint(&transcription.id,0,1,json!({"outputs":[{"index":0,"wavAssetId":wav.id,"wavHash":wav.content_hash,"jsonAssetId":json.id,"jsonHash":json.content_hash}]})).unwrap();
    storage
        .finish_job(&transcription.id, 1, "failed", Some("synthetic"))
        .unwrap();
    let diarization = retry(&storage, &meeting, "diarization");
    storage
        .acquire_job_kind(Some(JobKind::Diarization))
        .unwrap()
        .unwrap();
    let (spec, _) = storage.fixed_job_input(&diarization.id).unwrap();
    let params = &spec.parameters["diarization"];
    let track = &params["source"]["tracks"][0];
    let result = json!({"schemaVersion":1,"originOperationId":spec.parameters["request"]["operationId"],"inputAnalysisId":spec.input_analysis_id,"inputHash":spec.input_hash,"sourceManifestHash":params["source"]["manifestHash"],"trackId":track["id"],"trackStartUs":track["startUs"],"durationUs":4_000_000,"engine":params["engine"],"numSpeakers":3,"segments":[{"startUs":0,"endUs":1_000_000,"speaker":0}]});
    let dwav = artifact(
        &storage,
        &meeting,
        &diarization.id,
        "input.wav",
        "audio/wav",
        b"synthetic speaker PCM fixture",
    );
    let stdout = artifact(
        &storage,
        &meeting,
        &diarization.id,
        "stdout.log",
        "text/plain",
        b"private-native-log /Users/source-machine/secret-sherpa",
    );
    let result = artifact(
        &storage,
        &meeting,
        &diarization.id,
        "result.json",
        "application/json",
        &serde_json::to_vec(&result).unwrap(),
    );
    storage.save_job_checkpoint(&diarization.id,0,1,json!({"outputs":[{"index":0,"wavAssetId":dwav.id,"wavHash":dwav.content_hash,"stdoutAssetId":stdout.id,"stdoutHash":stdout.content_hash,"resultAssetId":result.id,"resultHash":result.content_hash}]})).unwrap();
    storage
        .finish_job(&diarization.id, 1, "failed", Some("synthetic"))
        .unwrap();
    let source = read(&storage, output.path());
    assert_eq!(source.manifest.omitted_assets.len(), 3);
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut target = project(&fixed);
    register_assets_except(&fixed, &target, &BTreeSet::from([result.id.clone()]));
    rebuild_analyses(&fixed, &mut target);
    let original_result = std::fs::read(&source.assets[&result.id]).unwrap();
    let rewritten = fixed
        .diarization_result(&meeting, &diarization.id, &original_result)
        .unwrap();
    let old: Value = serde_json::from_slice(&original_result).unwrap();
    let new: Value = serde_json::from_slice(&rewritten).unwrap();
    assert_ne!(new["inputAnalysisId"], old["inputAnalysisId"]);
    assert_ne!(new["inputHash"], old["inputHash"]);
    assert_ne!(new["trackId"], old["trackId"]);
    assert_eq!(new["segments"], old["segments"]);
    assert_eq!(new["engine"], old["engine"]);
    assert_eq!(new["durationUs"], old["durationUs"]);
    fixed.asset_bytes(&meeting, &result.id, &rewritten).unwrap();
    for key in [
        "originOperationId",
        "inputHash",
        "sourceManifestHash",
        "trackId",
    ] {
        let mut bad = old.clone();
        bad[key] = json!("forged");
        assert!(
            fixed
                .diarization_result(
                    &meeting,
                    &diarization.id,
                    &serde_json::to_vec(&bad).unwrap()
                )
                .is_err(),
            "{key}"
        );
    }
    let mut bad = old.clone();
    bad["segments"][0]["endUs"] = json!(5_000_000);
    assert!(fixed
        .diarization_result(
            &meeting,
            &diarization.id,
            &serde_json::to_vec(&bad).unwrap()
        )
        .is_err());
    for job in [&transcription, &diarization] {
        let spec = fixed.job_spec(&meeting, &job.id).unwrap();
        let parent = ids
            .get(
                "processing_jobs",
                &meeting,
                job.retry_of_job_id.as_deref().unwrap(),
            )
            .unwrap();
        let hash = fixed
            .job_hash(&meeting, &job.id, &spec, Some(&parent), job.attempt)
            .unwrap();
        let rows:Vec<(i64,i64)>=source.database.prepare("SELECT rowid,completed_units FROM job_checkpoints WHERE job_id=?1 ORDER BY sequence").unwrap().query_map(params![job.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
        for (rowid, units) in rows {
            let mapped = fixed
                .checkpoint(&meeting, &job.id, rowid, &|| false)
                .unwrap();
            let json: Value = serde_json::from_str(&mapped.json).unwrap();
            assert_eq!(json["outputs"].as_array().unwrap().len() as i64, units);
            assert_eq!(
                mapped.content_hash,
                super::super::super::super::jobs::checkpoint_hash(&hash, units, &mapped.json)
            );
            assert_eq!(
                mapped.missing_assets.len(),
                if units == 0 {
                    0
                } else if job.kind == "transcription" {
                    1
                } else {
                    2
                }
            );
            if units != 0 {
                assert_eq!(
                    json["outputs"][0]["wavAssetId"],
                    ids.get(
                        "file_assets",
                        &meeting,
                        if job.kind == "transcription" {
                            &wav.id
                        } else {
                            &dwav.id
                        }
                    )
                    .unwrap()
                );
                if job.kind == "diarization" {
                    assert_ne!(json["outputs"][0]["resultHash"], result.content_hash);
                }
            }
            assert!(fixed
                .checkpoint(&meeting, &job.id, rowid, &|| true)
                .is_err());
        }
    }
    assert!(source.assets.values().all(|path| path.exists()));
    assert!(storage.verify_asset(&stdout.id).is_ok());
    let (source_spec, source_checkpoint) = storage.fixed_job_input(&diarization.id).unwrap();
    assert_eq!(source_spec, spec);
    assert_eq!(
        source_checkpoint.unwrap()["outputs"][0]["resultAssetId"],
        result.id
    );
    let rowid: i64 = source
        .database
        .query_row(
            "SELECT rowid FROM job_checkpoints WHERE job_id=?1 ORDER BY sequence DESC LIMIT 1",
            params![transcription.id],
            |r| r.get(0),
        )
        .unwrap();
    let text: String = source
        .database
        .query_row(
            "SELECT checkpoint_json FROM job_checkpoints WHERE rowid=?1",
            params![rowid],
            |r| r.get(0),
        )
        .unwrap();
    source
        .database
        .execute_batch("PRAGMA query_only=OFF;")
        .unwrap();
    source
        .database
        .execute(
            "UPDATE job_checkpoints SET checkpoint_hash=?1 WHERE rowid=?2",
            params!["a".repeat(64), rowid],
        )
        .unwrap();
    source
        .database
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    assert!(fixed
        .checkpoint(&meeting, &transcription.id, rowid, &|| false)
        .err()
        .unwrap()
        .contains("変更"));
    let mut bad: Value = serde_json::from_str(&text).unwrap();
    bad["outputs"][0]["index"] = json!(1);
    let bad = bad.to_string();
    source
        .database
        .execute_batch("PRAGMA query_only=OFF;")
        .unwrap();
    source
        .database
        .execute(
            "UPDATE job_checkpoints SET checkpoint_json=?1,checkpoint_hash=?2 WHERE rowid=?3",
            params![
                bad,
                super::super::super::super::jobs::checkpoint_hash(
                    &transcription.spec_hash,
                    1,
                    &bad
                ),
                rowid
            ],
        )
        .unwrap();
    source
        .database
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    assert!(fixed
        .checkpoint(&meeting, &transcription.id, rowid, &|| false)
        .err()
        .unwrap()
        .contains("再開情報"));
}

#[test]
fn generated_item_references_are_scoped_to_the_exact_fixed_input_and_text_is_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "items");
    let source = read(&storage, output.path());
    let input: String = source
        .database
        .query_row(
            "SELECT input_analysis_id FROM processing_jobs WHERE kind='minutes_generation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (utterance,speaker):(String,String)=source.database.query_row("SELECT u.id,s.id FROM utterances u JOIN speakers s ON s.analysis_revision_id=u.analysis_revision_id WHERE u.analysis_revision_id=?1 LIMIT 1",params![input],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let old = json!([{"kind":"todo","body":format!("元発言IDを自然文に保持 {utterance}"),"assigneeName":"手動話者","assigneeSpeakerId":speaker,"dueDate":null,"dueText":"次回","evidenceUtteranceIds":[utterance]}]);
    let mapped: Value = serde_json::from_slice(
        &fixed
            .generation_items(&meeting, &input, &serde_json::to_vec(&old).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mapped[0]["body"], old[0]["body"]);
    assert_eq!(mapped[0]["dueText"], old[0]["dueText"]);
    assert_eq!(
        mapped[0]["evidenceUtteranceIds"][0],
        ids.get("utterances", &meeting, &utterance).unwrap()
    );
    assert_eq!(
        mapped[0]["assigneeSpeakerId"],
        ids.get("speakers", &meeting, &speaker).unwrap()
    );
    let base = storage.load_current_analysis(&meeting).unwrap().unwrap();
    assert!(fixed
        .generation_items(&meeting, &base.id, &serde_json::to_vec(&old).unwrap())
        .is_err());
    let mut invalid = old;
    invalid[0]["unknownReference"] = json!(utterance);
    assert!(fixed
        .generation_items(&meeting, &input, &serde_json::to_vec(&invalid).unwrap())
        .is_err());
}

fn generation_parameters() -> Value {
    json!({"schemaVersion":2,"templateVersion":"standard-v1","engine":{"executable":{"path":"mon-unavailable:sha","size":1,"sha256":"a".repeat(64)},"model":{"path":"mon-unavailable:sha","size":1,"sha256":"b".repeat(64)},"threads":1,"contextTokens":8192,"predictionTokens":512,"gpuLayers":0,"seed":1},"promptBudget":2048,"plan":{"nodes":[{"type":"extract","slices":[]},{"type":"extract","slices":[]},{"type":"merge","left":0,"right":1},{"type":"merge","left":0,"right":2}]}})
}

#[test]
fn generation_reference_walk_includes_completed_merges_and_pending_batches_using_worker_types() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "walker");
    let job = storage
        .list_jobs(&meeting)
        .unwrap()
        .into_iter()
        .find(|j| j.kind == "minutes_generation")
        .unwrap();
    let mut job = job;
    job.completed_units = 3;
    job.total_units = Some(4);
    let artifact = |name: &str, id: &str| json!({"name":name,"id":id,"hash":"a".repeat(64)});
    let batch = json!({"artifacts":[artifact("system.txt","system"),artifact("prompt.txt","prompt"),artifact("schema.json","schema"),artifact("result.json","raw")],"normalized":artifact("items.json","batch-items")});
    let checkpoint = json!({"outputs":[{"index":0,"artifacts":batch["artifacts"],"normalized":artifact("items.json","first-items")},{"index":1,"artifacts":batch["artifacts"],"normalized":artifact("items.json","second-items")},{"index":2,"artifacts":[],"normalized":artifact("items.json","merge-items"),"mergeBatches":[batch.clone()]}],"pending":{"index":3,"batches":[batch]}});
    let mut visited = vec![];
    let mapped = crate::generation::restore_checkpoint_references(
        &job,
        generation_parameters(),
        checkpoint.clone(),
        |role, id, _| {
            visited.push((role, id.to_owned()));
            Ok((format!("new-{id}"), "b".repeat(64)))
        },
    )
    .unwrap();
    assert_eq!(visited.len(), 21);
    assert_eq!(
        mapped["pending"]["batches"][0]["normalized"]["id"],
        "new-batch-items"
    );
    assert_eq!(
        mapped["outputs"][2]["mergeBatches"][0]["artifacts"][3]["hash"],
        "b".repeat(64)
    );
    let mut legacy = checkpoint.clone();
    legacy.as_object_mut().unwrap().remove("pending");
    legacy["outputs"][2]["artifacts"] = checkpoint["outputs"][0]["artifacts"].clone();
    legacy["outputs"][2]
        .as_object_mut()
        .unwrap()
        .remove("mergeBatches");
    let mut parameters = generation_parameters();
    parameters["schemaVersion"] = json!(1);
    let mut count = 0;
    crate::generation::restore_checkpoint_references(&job, parameters, legacy, |_, id, h| {
        count += 1;
        Ok((id.into(), h.into()))
    })
    .unwrap();
    assert_eq!(count, 15);
    let mut bad = checkpoint.clone();
    bad["pending"]["index"] = json!(2);
    assert!(crate::generation::restore_checkpoint_references(
        &job,
        generation_parameters(),
        bad,
        |_, _, _| panic!("reject before mapping")
    )
    .is_err());
    let mut bad = checkpoint;
    bad["outputs"][0]["normalized"]["name"] = json!("prompt.txt");
    assert!(crate::generation::restore_checkpoint_references(
        &job,
        generation_parameters(),
        bad,
        |_, id, h| Ok((id.into(), h.into()))
    )
    .is_err());
}
