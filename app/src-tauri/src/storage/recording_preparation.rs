use super::{now_ms, Meeting, Storage};
use rusqlite::params;
use uuid::Uuid;

impl Storage {
    // Called only inside the recording-start transition after live input/scope
    // admission. Opening or cancelling UI preparation never calls this method.
    // Existing IDs are rejected, not adopted or overwritten. Lost replies are
    // reconciled by reading the known ID; capture is never automatically replayed.
    pub(crate) fn create_recording_draft(&self, id: &str, title: &str) -> Result<Meeting, String> {
        self.require_data_writable()?;
        let parsed = Uuid::parse_str(id).map_err(|_| "録音準備の識別子が不正です")?;
        if parsed.get_version_num() != 4
            || parsed.to_string() != id
            || !matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
        {
            return Err("録音準備の識別子が不正です".into());
        }
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("会議名は1～200文字で入力してください".into());
        }
        let now = now_ms()?;
        let meeting = Meeting {
            id: id.to_owned(),
            title: title.to_owned(),
            lifecycle: "draft".into(),
            source_type: None,
            created_at: now,
            updated_at: now,
            lock_version: 0,
        };
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        // Keep the check and insert under the existing connection/authorizer.
        // The primary key is the final concurrency guard. No INSERT OR REPLACE.
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists {
            return Err(
                "録音準備の会議は既に保存されています。受付結果を確認してから操作してください"
                    .into(),
            );
        }
        db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,?2,'draft',?3,?3)", params![id,title,now]).map_err(|e| e.to_string())?;
        Ok(meeting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn exact_preparation_id_title_and_initial_version_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().to_owned()).unwrap();
        let id = Uuid::new_v4().to_string();
        let saved = storage
            .create_recording_draft(&id, "  2026/10/08 09:04の会議  ")
            .unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.title, "2026/10/08 09:04の会議");
        assert_eq!(saved.lifecycle, "draft");
        assert_eq!(saved.source_type, None);
        assert_eq!(saved.lock_version, 0);
        drop(storage);
        let reopened = Storage::open(dir.path().to_owned()).unwrap();
        assert_eq!(reopened.get_meeting(&id).unwrap(), Some(saved));
        assert_eq!(reopened.list_meetings().unwrap().len(), 1);
    }

    #[test]
    fn duplicate_or_changed_request_never_adopts_or_rewrites_existing_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().to_owned()).unwrap();
        let id = Uuid::new_v4().to_string();
        let original = storage.create_recording_draft(&id, "確認済み").unwrap();
        for title in ["確認済み", "別のタイトル"] {
            assert!(storage.create_recording_draft(&id, title).is_err());
        }
        assert_eq!(storage.get_meeting(&id).unwrap(), Some(original));
        let changed = storage.rename_meeting(&id, "変更済み", 0).unwrap();
        assert!(storage.create_recording_draft(&id, "確認済み").is_err());
        assert_eq!(storage.get_meeting(&id).unwrap(), Some(changed));
    }

    #[test]
    fn invalid_identity_or_title_leaves_no_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().to_owned()).unwrap();
        let id = Uuid::new_v4().to_string();
        for invalid in [
            "".to_owned(),
            id.to_uppercase(),
            "not-uuid".into(),
            "735027d4-a03e-13d3-830a-21e767dfaf88".into(),
        ] {
            assert!(storage.create_recording_draft(&invalid, "会議").is_err());
        }
        for title in ["  \n ".to_owned(), "😀".repeat(201)] {
            assert!(storage.create_recording_draft(&id, &title).is_err());
        }
        assert!(storage.list_meetings().unwrap().is_empty());
        assert!(storage
            .create_recording_draft(&id, &"😀".repeat(200))
            .is_ok());
    }

    #[test]
    fn retired_or_database_failure_cannot_publish_new_preparation() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().to_owned()).unwrap();
        let id = Uuid::new_v4().to_string();
        storage.clear_analysis_pcm();
        storage.retired.store(true, Ordering::Release);
        assert!(storage
            .create_recording_draft(&id, "停止済み保存先")
            .is_err());
        storage.retired.store(false, Ordering::Release);
        storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_prepared_meeting BEFORE INSERT ON meetings BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
        assert!(storage.create_recording_draft(&id, "作成失敗").is_err());
        assert!(storage.get_meeting(&id).unwrap().is_none());
        storage
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_prepared_meeting")
            .unwrap();
        assert!(storage.create_recording_draft(&id, "明示再確認").is_ok());
    }

    #[test]
    fn concurrent_same_id_requests_create_exactly_one_draft_without_capture() {
        let dir = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(Storage::open(dir.path().to_owned()).unwrap());
        let id = Uuid::new_v4().to_string();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let storage = storage.clone();
                let id = id.clone();
                std::thread::spawn(move || storage.create_recording_draft(&id, "確認済み"))
            })
            .collect();
        let successes = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap().ok())
            .collect::<Vec<_>>();
        assert_eq!(successes.len(), 1);
        assert_eq!(storage.list_meetings().unwrap(), successes);
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM recording_sessions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn confirmed_draft_flows_into_existing_recording_storage_without_another_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().to_owned()).unwrap();
        let id = Uuid::new_v4().to_string();
        storage.create_recording_draft(&id, "確認済み").unwrap();
        let config = super::super::RecordingConfig {
            mode: "system".into(),
            backend: "screencapturekit".into(),
            target_pids: vec![],
            microphone_device_uid: None,
            target_application_identity: None,
        };
        let session = storage
            .prepare_recording(&id, &config, "confirmed-preparation")
            .unwrap();
        assert_eq!(session.meeting_id, id);
        assert_eq!(storage.list_meetings().unwrap().len(), 1);
        let saved = storage.get_meeting(&id).unwrap().unwrap();
        assert_eq!(saved.lifecycle, "recording");
        assert_eq!(saved.source_type.as_deref(), Some("recording"));
        assert!(storage.create_recording_draft(&id, "確認済み").is_err());
    }
}
