//! Test-only seeding and inspection of previously stored structured minutes.
//! These helpers are not product edit/adopt/restore commands. They populate
//! isolated temporary databases for migration, backup and deletion tests.
use super::*;
use crate::minutes_document::{from_generated, GeneratedItem, ItemKind, MinutesContent};
use serde_json::json;

pub(in crate::storage) fn meeting(storage: &Storage) -> String {
    let m = super::analysis::tests::saved_meeting(storage);
    let mut input = super::analysis::tests::input();
    input.utterances[0].text = "点検日を確認することに決まりました。".into();
    let id = storage.create_analysis_candidate(&m.id, input).unwrap();
    storage
        .commit_analysis_candidate(&m.id, &id, 0, false)
        .unwrap();
    storage.create_transcript_draft(&m.id, &id).unwrap();
    m.id
}
pub(in crate::storage) fn candidate(storage: &Storage, meeting: &str) -> String {
    let analysis = storage.load_current_analysis(meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(meeting).unwrap().unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.into(),
        operation_id: Uuid::new_v4().to_string(),
        expected_analysis_id: analysis.id,
        expected_content_hash: analysis.content_hash,
        expected_meeting_lock_version: storage.get_meeting(meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        }),
    };
    let job = storage
        .enqueue_minutes_job(
            request,
            json!({"engine":"fixture"}),
            json!({"templateVersion":"standard-v1"}),
            Some(1),
        )
        .unwrap();
    storage.acquire_local_inference_job().unwrap().unwrap();
    storage
        .save_job_checkpoint(&job.id, 0, 1, json!({"fixture":true}))
        .unwrap();
    let input = storage.load_minutes_input(&job.id).unwrap();
    let content = from_generated(
        &input,
        vec![GeneratedItem {
            kind: ItemKind::Decision,
            body: "点検日を確認する".into(),
            assignee_name: None,
            assignee_speaker_id: None,
            due_date: None,
            due_text: None,
            evidence_utterance_ids: vec![input.analysis.utterances[0].id.clone()],
        }],
    )
    .unwrap();
    storage
        .finish_minutes_candidate(&job.id, 1, content)
        .unwrap()
}

#[derive(Serialize)]
pub(super) struct StoredState {
    pub current_id: Option<String>,
    pub meeting_lock_version: i64,
    pub draft: Option<DraftStamp>,
    pub versions: Vec<StoredRevision>,
}
#[derive(Serialize)]
pub(super) struct StoredRevision {
    pub id: String,
    pub publication_state: String,
    pub input_updated: bool,
}

impl Storage {
    // Historical receipt seeding only. Never opens an external path or
    // reinstates the deleted structured-minutes export commands.
    pub(super) fn seed_stored_export_receipt(
        &self,
        meeting: &str,
        revision: &str,
        draft: &MinutesDraft,
        path: &Path,
        state: &str,
        output: &[u8],
    ) -> ExportRecord {
        assert!(matches!(state, "succeeded" | "failed"));
        let id = Uuid::new_v4().to_string();
        let now = now_ms().unwrap();
        let content = serde_json::to_vec(&draft.payload).unwrap();
        let request_hash = format!("{:x}", Sha256::digest(format!("stored-receipt:{id}")));
        let content_hash = format!("{:x}", Sha256::digest(content));
        let output_hash = format!("{:x}", Sha256::digest(output));
        let db = self.connection.lock().unwrap();
        db.execute("INSERT INTO export_records(id,meeting_id,minutes_revision_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,source_draft_id,source_draft_lock_version,format,destination_path,state,error_message,created_at,updated_at,exported_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'markdown',?10,?11,?12,?13,?13,?14)",params![id,meeting,revision,request_hash,content_hash,output_hash,output.len() as i64,draft.id,draft.lock_version,path.to_str().unwrap(),state,if state=="failed"{Some("stored export failure")}else{None},now,if state=="succeeded"{Some(now)}else{None}]).unwrap();
        db.query_row(
            &format!(
                "SELECT {} FROM export_records WHERE id=?1",
                super::exports::COLUMNS
            ),
            [id],
            super::exports::record,
        )
        .unwrap()
    }

    pub(super) fn seed_stored_minutes_head(
        &self,
        meeting: &str,
        revision: &str,
    ) -> Result<(), String> {
        let mut db = self.connection.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let hash = super::revisions::minutes_content_hash(&tx, revision)?;
        tx.execute("UPDATE minutes_revisions SET publication_state='committed',content_hash=?1 WHERE id=?2 AND meeting_id=?3", params![hash,revision,meeting]).map_err(|e| e.to_string())?;
        tx.execute("UPDATE meetings SET previous_minutes_id=current_minutes_id,current_minutes_id=?1,lock_version=lock_version+1,updated_at=?2 WHERE id=?3", params![revision,now_ms()?,meeting]).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE working_drafts SET active=0 WHERE meeting_id=?1 AND target_kind='minutes'",
            [meeting],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE working_drafts SET active=1 WHERE meeting_id=?1 AND target_kind='minutes' AND base_minutes_id=?2", params![meeting,revision]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub(super) fn seed_stored_minutes_draft(
        &self,
        meeting: &str,
        base: &str,
    ) -> Result<MinutesDraft, String> {
        let mut db = self.connection.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        if let Some(draft) = super::minutes_drafts::load(&tx, meeting, Some(base))? {
            return Ok(draft);
        }
        let view =
            super::minutes::read_content(&tx, meeting, base)?.ok_or("fixture base missing")?;
        let text = serde_json::to_string(&view.content).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms()?;
        tx.execute("INSERT INTO working_drafts (id,meeting_id,target_kind,base_minutes_id,payload_json,schema_version,content_hash,lock_version,created_at,updated_at) VALUES (?1,?2,'minutes',?3,?4,1,?5,0,?6,?6)", params![id,meeting,base,text,super::minutes_drafts::hash(&text),now]).map_err(|e| e.to_string())?;
        let draft = super::minutes_drafts::load(&tx, meeting, Some(base))?.unwrap();
        tx.commit().map_err(|e| e.to_string())?;
        Ok(draft)
    }

    // Seed an existing edit/history pair, so restore verification still covers
    // hashes and inverse payloads without retaining a product rich editor.
    pub(super) fn seed_stored_minutes_edit(
        &self,
        id: &str,
        expected: i64,
        payload: MinutesContent,
    ) -> Result<MinutesDraft, String> {
        let mut db = self.connection.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (meeting, old, lock): (String, String, i64) = tx
            .query_row(
                "SELECT meeting_id,payload_json,lock_version FROM working_drafts WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        assert_eq!(lock, expected, "fixture edit version");
        let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        if old != text {
            let now = now_ms()?;
            tx.execute("INSERT INTO edit_operations (id,draft_id,sequence,operation_kind,payload_json,inverse_json,before_hash,after_hash,created_at) VALUES (?1,?2,(SELECT COALESCE(MAX(sequence),0)+1 FROM edit_operations WHERE draft_id=?2),'replace_minutes',?3,?4,?5,?6,?7)", params![Uuid::new_v4().to_string(),id,text,old,super::minutes_drafts::hash(&old),super::minutes_drafts::hash(&text),now]).map_err(|e| e.to_string())?;
            tx.execute("UPDATE working_drafts SET payload_json=?1,content_hash=?2,lock_version=lock_version+1,updated_at=?3 WHERE id=?4", params![text,super::minutes_drafts::hash(&text),now,id]).map_err(|e| e.to_string())?;
        }
        let draft = super::minutes_drafts::load(&tx, &meeting, None)?.unwrap();
        tx.commit().map_err(|e| e.to_string())?;
        Ok(draft)
    }

    pub(super) fn read_stored_minutes_draft(
        &self,
        meeting: &str,
        base: Option<&str>,
    ) -> Result<Option<MinutesDraft>, String> {
        let mut db = self.connection.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        super::minutes_drafts::load(&tx, meeting, base)
    }

    pub(super) fn read_stored_minutes_state(&self, meeting: &str) -> Result<StoredState, String> {
        let mut db = self.connection.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (current_id, meeting_lock_version): (Option<String>, i64) = tx
            .query_row(
                "SELECT current_minutes_id,lock_version FROM meetings WHERE id=?1",
                [meeting],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let draft = super::minutes_drafts::load(&tx, meeting, None)?.map(|d| DraftStamp {
            id: d.id,
            lock_version: d.lock_version,
        });
        let mut query = tx.prepare("SELECT id,publication_state FROM minutes_revisions WHERE meeting_id=?1 AND (publication_state='candidate' OR id IN (SELECT current_minutes_id FROM meetings WHERE id=?1) OR id IN (SELECT previous_minutes_id FROM meetings WHERE id=?1)) ORDER BY revision_no DESC").map_err(|e| e.to_string())?;
        let rows = query
            .query_map([meeting], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut versions = vec![];
        for row in rows {
            let (id, publication_state) = row.map_err(|e| e.to_string())?;
            let view = super::minutes::read_content(&tx, meeting, &id)?.unwrap();
            let input = super::minutes_input::read_snapshot(
                &tx,
                meeting,
                &view.input_analysis_id,
                &view.input_content_hash,
            )?;
            versions.push(StoredRevision {
                id,
                publication_state,
                input_updated: super::minutes_input::input_updated(&tx, meeting, &input)?,
            });
        }
        Ok(StoredState {
            current_id,
            meeting_lock_version,
            draft,
            versions,
        })
    }
}
