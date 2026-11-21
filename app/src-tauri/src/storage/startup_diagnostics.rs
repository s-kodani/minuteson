use super::{deletion, meeting_access, Meeting, Storage};
use rusqlite::{Connection, Row};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Diagnostic {
    meeting_id: Option<String>,
    reason: String,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeetingListDiagnostics {
    items: Vec<Diagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageWriteState {
    read_only: bool,
    reasons: Vec<String>,
    pub(crate) salvage_available: bool,
}

#[derive(Default)]
pub(super) struct Diagnostics(Mutex<BTreeMap<(Option<String>, String), Diagnostic>>);

impl Diagnostics {
    pub(super) fn record(&self, meeting_id: Option<&str>, reason: &str, message: &str) {
        let entry = diagnostic(meeting_id, reason, message);
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert((entry.meeting_id.clone(), entry.reason.clone()), entry);
    }
}

impl Storage {
    pub(crate) fn startup_diagnostic(&self, meeting_id: Option<&str>, reason: &str, message: &str) {
        self.startup_diagnostics.record(meeting_id, reason, message);
    }

    pub(crate) fn clear_startup_diagnostic(&self, meeting: &str, reason: &str) {
        if let Ok(mut entries) = self.startup_diagnostics.0.lock() {
            entries.remove(&(Some(meeting.into()), reason.into()));
        }
    }

    pub(crate) fn storage_write_state(&self) -> StorageWriteState {
        let mut reasons = self.startup_write_holds.clone();
        if reasons.is_empty() {
            reasons.extend(self.require_data_writable().err());
        }
        StorageWriteState {
            read_only: self.data_write_blocked(),
            reasons,
            salvage_available: self.ready_salvage_available(),
        }
    }

    pub(crate) fn ready_salvage_available(&self) -> bool {
        if !cfg!(any(target_os = "macos", windows))
            || self.recovery_hold
            || self.restart_required()
            || self.maintenance.load(Ordering::Acquire)
        {
            return false;
        }
        let persistent_failure = if self.deletion_fence.pending() {
            self.startup_write_holds.is_empty()
                && self.deletion_fence.conflicting_shared_startup_target()
        } else {
            self.startup_shared_database_corrupt
        };
        if !persistent_failure {
            return false;
        }
        // No normal restore or other store maintenance may be bypassed even
        // when independent damage prevents a deletion target from being chosen.
        self.connection.lock().is_ok_and(|db| {
            db.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM maintenance_operations
                 WHERE kind NOT IN ('delete_meeting','delete_assets')
                 AND state NOT IN ('completed','recovered'))",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false)
        })
    }

    pub(crate) fn meeting_list_diagnostics(&self) -> Result<MeetingListDiagnostics, String> {
        let (_, mut items) = self.meeting_list()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        protected_diagnostics(&db, &mut items)?;
        let sources = super::meeting_audio::availability::Source::ready(&db)?;
        drop(db);
        let protected = items
            .iter()
            .filter(|item| {
                matches!(
                    item.reason.as_str(),
                    "deletion_pending" | "audio_deletion_pending" | "audio_unavailable"
                )
            })
            .filter_map(|item| item.meeting_id.as_deref())
            .collect::<std::collections::HashSet<_>>();
        let mut audio_issues = Vec::new();
        let mut pending_audio = deletion::fence::AudioPendingScopes::default();
        for candidate in sources {
            if candidate
                .meeting
                .as_deref()
                .is_some_and(|id| protected.contains(id))
            {
                continue;
            }
            let source = match candidate.source {
                Ok(source) => source,
                Err(error) => {
                    let issue = super::meeting_audio::availability::Issue::Unreadable(error);
                    audio_issues.push(diagnostic(
                        candidate.meeting.as_deref(),
                        issue.reason(),
                        &issue.message(),
                    ));
                    continue;
                }
            };
            if let super::meeting_audio::availability::Observation::Unavailable(issue) =
                source.inspect_with_scopes(self, &mut pending_audio)?
            {
                audio_issues.push(diagnostic(
                    Some(&source.meeting),
                    issue.reason(),
                    &issue.message(),
                ));
            }
        }
        items.extend(audio_issues);
        let entries = self
            .startup_diagnostics
            .0
            .lock()
            .map_err(|_| "起動時の診断を読み込めません")?;
        items.extend(entries.values().cloned());
        items.sort_by(|a, b| (&a.meeting_id, &a.reason).cmp(&(&b.meeting_id, &b.reason)));
        items.dedup_by(|a, b| a.meeting_id == b.meeting_id && a.reason == b.reason);
        Ok(MeetingListDiagnostics { items })
    }

    pub(super) fn meeting_list(&self) -> Result<(Vec<Meeting>, Vec<Diagnostic>), String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut query = db.prepare("SELECT id,title,lifecycle,created_at,updated_at,lock_version,source_type FROM meetings ORDER BY updated_at DESC,id DESC").map_err(|error| error.to_string())?;
        let mut rows = query.query([]).map_err(|error| error.to_string())?;
        let mut meetings = Vec::new();
        let mut issues = Vec::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            match meeting_row(row) {
                Ok((meeting, invalid)) => {
                    if invalid {
                        issues.push(diagnostic(
                            Some(&meeting.id),
                            "meeting_row_invalid",
                            "会議の一覧情報の一部を読み取れません。会議データを保持しています",
                        ));
                    }
                    meetings.push(meeting);
                }
                Err(_) => issues.push(diagnostic(
                    None,
                    "meeting_row_invalid",
                    "識別子を読み取れない会議があります。会議データを保持しています",
                )),
            }
        }
        Ok((meetings, issues))
    }
}

pub(super) fn structural_database_error(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
    )
}

fn protected_diagnostics(db: &Connection, issues: &mut Vec<Diagnostic>) -> Result<(), String> {
    // Query only the protected resources; a large deletion history must
    // not be revalidated once for every meeting appearing in the list.
    if meeting_access::installed(db)? {
        let mut guarded = db
            .prepare("SELECT meeting_id FROM temp.mon_meeting_deletion_guard")
            .map_err(|error| error.to_string())?;
        let ids = guarded
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for id in ids {
            issues.push(diagnostic(
                Some(&id.map_err(|error| error.to_string())?),
                "deletion_pending",
                "この会議は起動時の削除確認のため書込みを停止しています",
            ));
        }
    }
    audio_diagnostics(db, issues)?;
    let salvage_present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='salvage_meetings')",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
    if salvage_present {
        let mut query = db
            .prepare("SELECT meeting_id FROM salvage_meetings")
            .map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for id in rows.filter_map(Result::ok) {
            match super::salvage_schema::meeting(db, &id) {
                Ok(Some(record)) => {
                    if let Some(reason) = record.audio_unavailable {
                        issues.push(diagnostic(Some(&id), "audio_unavailable", &reason));
                    }
                    for (part, reason) in record.notes {
                        issues.push(diagnostic(Some(&id), &format!("salvage_{part}"), &reason));
                    }
                }
                Ok(None) => {}
                Err(error) => issues.push(diagnostic(Some(&id), "salvage_record_invalid", &error)),
            }
        }
    }
    Ok(())
}

fn audio_diagnostics(db: &Connection, issues: &mut Vec<Diagnostic>) -> Result<(), String> {
    let pending = deletion::fence::predicate(db)?;
    let mut query = db
        .prepare(&format!(
            "SELECT DISTINCT coalesce(meeting_id,{}) FROM maintenance_operations
         WHERE kind='delete_assets' AND {pending}",
            deletion::fence::MANIFEST_AUDIO_MEETING,
        ))
        .map_err(|error| error.to_string())?;
    for id in query
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .map_err(|error| error.to_string())?
    {
        if let Some(id) = id.ok().flatten() {
            issues.push(diagnostic(
                Some(&id),
                "audio_deletion_pending",
                "音声の削除確認が必要です。文字起こし・議事録は保持しています",
            ));
        }
    }
    Ok(())
}

pub(super) fn meeting_row(row: &Row<'_>) -> rusqlite::Result<(Meeting, bool)> {
    let id: String = row.get(0)?;
    let title = row.get::<_, String>(1);
    let lifecycle = row.get::<_, String>(2);
    let created_at = row.get::<_, i64>(3);
    let updated_at = row.get::<_, i64>(4);
    let lock_version = row.get::<_, i64>(5);
    let source_type = row.get::<_, Option<String>>(6);
    let invalid = title.is_err()
        || lifecycle.is_err()
        || created_at.is_err()
        || updated_at.is_err()
        || lock_version.is_err()
        || source_type.is_err();
    Ok((
        Meeting {
            id,
            title: title.unwrap_or_else(|_| "会議名を確認できません".into()),
            lifecycle: lifecycle.unwrap_or_else(|_| "draft".into()),
            source_type: source_type.unwrap_or(None),
            created_at: created_at.unwrap_or_default(),
            updated_at: updated_at.unwrap_or_default(),
            lock_version: lock_version.unwrap_or_default(),
        },
        invalid,
    ))
}

fn diagnostic(meeting_id: Option<&str>, reason: &str, message: &str) -> Diagnostic {
    // Avoid reflecting arbitrarily-sized invalid input into the WebView.
    Diagnostic {
        meeting_id: meeting_id.map(str::to_owned),
        reason: reason.into(),
        message: message.chars().take(1024).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::with_test_database;
    use rusqlite::params;

    #[test]
    fn malformed_list_row_is_visible_beside_the_normal_meeting() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().join("data")).unwrap();
        let ordinary = storage.create_meeting("正常な会議").unwrap();
        let malformed = storage.create_meeting("一覧情報が不正な会議").unwrap();
        with_test_database(&storage, |db| {
            db.execute(
                "UPDATE meetings SET title=X'8081' WHERE id=?1",
                [&malformed.id],
            )
            .unwrap();
        });
        let meetings = storage.list_meetings().unwrap();
        assert!(meetings.contains(&ordinary));
        assert!(meetings.iter().any(|meeting| meeting.id == malformed.id));
        assert!(storage
            .meeting_list_diagnostics()
            .unwrap()
            .items
            .iter()
            .any(|item| item.meeting_id.as_deref() == Some(&malformed.id)
                && item.reason == "meeting_row_invalid"));
        assert!(storage.get_meeting(&malformed.id).unwrap().is_some());
        assert!(!storage.storage_write_state().salvage_available);
    }

    #[test]
    fn only_structural_sqlite_errors_are_salvage_evidence() {
        for (code, expected) in [
            (rusqlite::ffi::SQLITE_CORRUPT, true),
            (rusqlite::ffi::SQLITE_NOTADB, true),
            (rusqlite::ffi::SQLITE_BUSY, false),
            (rusqlite::ffi::SQLITE_LOCKED, false),
            (rusqlite::ffi::SQLITE_IOERR, false),
            (rusqlite::ffi::SQLITE_FULL, false),
            (rusqlite::ffi::SQLITE_CONSTRAINT, false),
        ] {
            let error = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
            assert_eq!(structural_database_error(&error), expected, "{error}");
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn conflicting_target_claims_leave_titles_readable_and_reject_mutations() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let storage = Storage::open(data.clone()).unwrap();
        let a = storage.create_meeting("会議A").unwrap();
        let b = storage.create_meeting("会議B").unwrap();
        with_test_database(&storage, |db| {
            let other_claim = serde_json::json!({"scope": {"meetingId": b.id}}).to_string();
            db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                        VALUES('unverified',?1,'delete_meeting','attention',?2,?3,'{}',0,0)",
                params![a.id, other_claim, "a".repeat(64)]).unwrap();
        });
        drop(storage);
        let storage = Storage::open(data).unwrap();
        assert!(storage.common_write_hold());
        assert_eq!(
            storage.storage_write_state().salvage_available,
            cfg!(any(target_os = "macos", windows))
        );
        assert_eq!(storage.list_meetings().unwrap().len(), 2);
        assert!(storage.rename_meeting(&a.id, "書込み不可", 0).is_err());
        assert!(storage.rename_meeting(&b.id, "書込み不可", 0).is_err());
        assert!(storage.create_meeting("書込み不可").is_err());
        assert!(storage.require_global_data_writable().is_err());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn another_pending_operation_takes_priority_over_a_shared_deletion_conflict() {
        for (kind, manifest) in [
            ("delete_assets", r#"{"subtype":"modelDeletion"}"#),
            ("move_storage", "{}"),
            ("delete_assets", "{}"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("data");
            let storage = Storage::open(data.clone()).unwrap();
            let a = storage.create_meeting("会議A").unwrap();
            let b = storage.create_meeting("会議B").unwrap();
            with_test_database(&storage, |db| {
                let other_claim = serde_json::json!({"scope": {"meetingId": b.id}}).to_string();
                db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                            VALUES('conflict',?1,'delete_meeting','attention',?2,?3,'{}',0,0)",
                    params![a.id, other_claim, "a".repeat(64)]).unwrap();
                db.execute("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                            VALUES('another',?1,'attention',?2,?3,'{}',0,0)",
                    params![kind, manifest, "a".repeat(64)]).unwrap();
            });
            drop(storage);
            let storage = Storage::open(data).unwrap();
            let state = storage.storage_write_state();
            assert!(state.read_only);
            assert!(!state.salvage_available, "{kind}: {manifest}");
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn pending_audio_deletion_holds_audio_and_allows_minutes_edits() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let storage = Storage::open(data.clone()).unwrap();
        let meeting = super::super::current_minutes_fixture::meeting(&storage);
        super::super::current_minutes_fixture::save(&storage, &meeting, "# 正常な議事録");
        with_test_database(&storage, |db| {
            let header = serde_json::json!({"subtype":"audioOnlyDeletion","meetingId":meeting});
            db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                        VALUES('unverified',?1,'delete_assets','attention',?2,?3,'{}',0,0)",
                params![meeting, header.to_string(), "a".repeat(64)]).unwrap();
        });
        drop(storage);
        let storage = Storage::open(data).unwrap();
        assert!(!storage.data_write_blocked());
        assert!(!storage.storage_write_state().salvage_available);
        assert!(
            with_test_database(&storage, |db| deletion::audio::require_source(db, &meeting))
                .is_err()
        );
        assert!(storage
            .meeting_list_diagnostics()
            .unwrap()
            .items
            .iter()
            .any(|item| item.meeting_id.as_deref() == Some(&meeting)
                && item.reason == "audio_deletion_pending"));
        super::super::current_minutes_fixture::save(&storage, &meeting, "# 編集を保存");
        storage.create_meeting("別の会議").unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn more_than_ten_thousand_completed_histories_are_retained_and_startup_remains_readable() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let storage = Storage::open(data.clone()).unwrap();
        let a = storage.create_meeting("正常な会議").unwrap();
        with_test_database(&storage, |db| {
            let mut record = db.prepare("INSERT INTO maintenance_operations(id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                        VALUES(?1,'delete_meeting','completed','{}',?2,'{}',0,0)").unwrap();
            let hash = "a".repeat(64);
            db.execute_batch("BEGIN").unwrap();
            for index in 0..10_001 {
                let id = uuid::Uuid::from_u128(index + 1).to_string();
                record.execute(params![id, hash]).unwrap();
            }
            record
                .execute(params!["invalid-operation-id", hash])
                .unwrap();
            db.execute_batch("COMMIT").unwrap();
        });
        drop(storage);
        let storage = Storage::open(data).unwrap();
        assert_eq!(storage.list_meetings().unwrap(), vec![a]);
        assert!(storage.storage_write_state().read_only);
        assert!(!storage.storage_write_state().salvage_available);
        with_test_database(&storage, |db| {
            let count = db
                .query_row("SELECT count(*) FROM maintenance_operations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(count, 10_002);
        });
    }
}
