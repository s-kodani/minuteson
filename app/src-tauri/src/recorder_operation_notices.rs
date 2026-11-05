//! In-process UI diagnostics only. Dismissing a toast never resolves its cause.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager, WebviewWindow};

const MAX_REVISION: u64 = 9_007_199_254_740_991;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    WindowAction,
    Confirmation,
}
const SOURCES: [Source; 2] = [Source::WindowAction, Source::Confirmation];
impl Source {
    fn index(self) -> usize {
        SOURCES.iter().position(|s| *s == self).unwrap()
    }
    fn key(self) -> &'static str {
        match self {
            Self::WindowAction => "window_action",
            Self::Confirmation => "confirmation",
        }
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub source: Source,
    pub revision: u64,
    pub event_id: Option<String>,
    pub message: Option<String>,
    pub dismissed: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    schema_version: u8,
    entries: Vec<Entry>,
}
pub struct Notices(Mutex<[Entry; 2]>);
impl Default for Notices {
    fn default() -> Self {
        Self(Mutex::new(SOURCES.map(|source| Entry {
            source,
            revision: 0,
            event_id: None,
            message: None,
            dismissed: false,
        })))
    }
}
impl Notices {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            schema_version: 1,
            entries: self.0.lock().unwrap_or_else(|e| e.into_inner()).to_vec(),
        }
    }
    fn update(
        &self,
        source: Source,
        expected: u64,
        message: Option<String>,
    ) -> Result<Snapshot, String> {
        if message
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 8192 || s.contains('\0'))
        {
            return Err("操作エラーの説明が空か長すぎます".into());
        }
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = &mut entries[source.index()];
        if entry.revision != expected {
            return Err("操作通知が変更されています。状態を読み直してください".into());
        }
        if entry.message != message {
            let next = entry
                .revision
                .checked_add(1)
                .filter(|n| *n <= MAX_REVISION)
                .ok_or("操作通知の版が上限に達しました")?;
            entry.revision = next;
            entry.event_id = message.as_ref().map(|_| format!("{}:{next}", source.key()));
            entry.message = message;
            entry.dismissed = false;
        }
        Ok(Snapshot {
            schema_version: 1,
            entries: entries.to_vec(),
        })
    }
    fn dismiss(&self, source: Source, expected: u64, event_id: &str) -> Result<Snapshot, String> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = &mut entries[source.index()];
        if entry.revision != expected
            || entry.event_id.as_deref() != Some(event_id)
            || entry.message.is_none()
        {
            return Err("閉じる対象の通知が変更されています".into());
        }
        if !entry.dismissed {
            entry.revision = entry
                .revision
                .checked_add(1)
                .filter(|n| *n <= MAX_REVISION)
                .ok_or("操作通知の版が上限に達しました")?;
            entry.dismissed = true;
        }
        Ok(Snapshot {
            schema_version: 1,
            entries: entries.to_vec(),
        })
    }
}
fn can_read(label: &str) -> bool {
    matches!(label, "main" | "recorder" | "recording-confirmation")
}
fn can_publish(label: &str, source: Source) -> bool {
    label == "recorder" || (label == "recording-confirmation" && source == Source::Confirmation)
}
fn can_dismiss(label: &str) -> bool {
    label == "main"
}
#[tauri::command]
pub fn get_recorder_operation_notices(
    window: WebviewWindow,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> = (|| {
        if !can_read(window.label()) {
            return Err("録音の操作画面から通知を確認してください".into());
        }
        Ok(window.state::<Notices>().snapshot())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_recorder_operation_notices", message)
    })
}
#[tauri::command]
pub fn publish_recorder_operation_notice(
    window: WebviewWindow,
    source: Source,
    expected_revision: u64,
    message: Option<String>,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> = (|| {
        if !can_publish(window.label(), source) {
            return Err("この録音画面からは対象の操作エラーを報告できません".into());
        }
        let value = window
            .state::<Notices>()
            .update(source, expected_revision, message)?;
        // Events are hints only; every reader checks the authoritative snapshot.
        let _ = window
            .app_handle()
            .emit("recorder-operation-notices-changed", ());
        Ok(value)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("publish_recorder_operation_notice", message)
    })
}
#[tauri::command]
pub fn dismiss_recorder_operation_notice(
    window: WebviewWindow,
    source: Source,
    expected_revision: u64,
    event_id: String,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> = (|| {
        if !can_dismiss(window.label()) {
            return Err("メイン画面の通知から閉じてください".into());
        }
        let value = window
            .state::<Notices>()
            .dismiss(source, expected_revision, &event_id)?;
        let _ = window
            .app_handle()
            .emit("recorder-operation-notices-changed", ());
        Ok(value)
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("dismiss_recorder_operation_notice", message)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dismissal_retains_cause_and_same_heartbeat_does_not_recreate_notification() {
        let n = Notices::default();
        let a = n
            .update(Source::WindowAction, 0, Some("開けません".into()))
            .unwrap()
            .entries[0]
            .clone();
        let b = n
            .dismiss(a.source, a.revision, a.event_id.as_deref().unwrap())
            .unwrap()
            .entries[0]
            .clone();
        assert!(b.dismissed);
        assert_eq!(b.message, a.message);
        assert_eq!(b.event_id, a.event_id);
        let same = n.update(a.source, b.revision, a.message).unwrap().entries[0].clone();
        assert_eq!(same, b);
    }
    #[test]
    fn stale_close_and_stale_resolution_never_clear_a_new_cause() {
        let n = Notices::default();
        n.update(Source::Confirmation, 0, Some("前の失敗".into()))
            .unwrap();
        let fresh = n
            .update(Source::Confirmation, 1, Some("新しい失敗".into()))
            .unwrap()
            .entries[1]
            .clone();
        assert!(n.update(Source::Confirmation, 1, None).is_err());
        assert!(n
            .dismiss(Source::Confirmation, 1, "confirmation:1")
            .is_err());
        assert_eq!(n.snapshot().entries[1], fresh);
        let clear = n
            .update(Source::Confirmation, fresh.revision, None)
            .unwrap()
            .entries[1]
            .clone();
        assert!(clear.message.is_none());
        assert!(clear.event_id.is_none());
        assert!(!clear.dismissed);
        assert!(n
            .update(Source::Confirmation, 0, Some("古い応答".into()))
            .is_err());
    }
    #[test]
    fn fixed_sources_are_independent_and_reader_roles_are_bounded() {
        let n = Notices::default();
        for s in SOURCES {
            n.update(s, 0, Some(s.key().into())).unwrap();
        }
        assert_eq!(n.snapshot().entries.len(), 2);
        for label in ["main", "recorder", "recording-confirmation"] {
            assert!(can_read(label));
        }
        for label in ["unknown", ""] {
            assert!(!can_read(label));
        }
        for label in ["main", "recorder", "recording-confirmation", "unknown"] {
            for source in SOURCES {
                assert_eq!(
                    can_publish(label, source),
                    label == "recorder"
                        || (label == "recording-confirmation" && source == Source::Confirmation)
                );
            }
            assert_eq!(can_dismiss(label), label == "main");
        }
    }
    #[test]
    fn oversized_blank_nul_and_exhausted_revisions_leave_snapshot_unchanged() {
        let n = Notices::default();
        for message in [" ".into(), "x".repeat(8193), "broken\0text".into()] {
            assert!(n.update(Source::WindowAction, 0, Some(message)).is_err());
        }
        assert_eq!(n.snapshot().entries[0].revision, 0);
        n.0.lock().unwrap()[0].revision = MAX_REVISION;
        assert!(n
            .update(Source::WindowAction, MAX_REVISION, Some("失敗".into()))
            .is_err());
        assert!(n.snapshot().entries[0].message.is_none());
    }
    #[test]
    fn wire_snapshot_uses_exact_camel_case_and_unknown_sources_are_rejected() {
        let n = Notices::default();
        let json = serde_json::to_value(n.snapshot()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["entries"].as_array().unwrap().len(), 2);
        assert_eq!(json["entries"][1]["source"], "confirmation");
        assert!(json["entries"][1]["eventId"].is_null());
        assert!(serde_json::from_str::<Source>("\"capture_write\"").is_err());
    }
}
