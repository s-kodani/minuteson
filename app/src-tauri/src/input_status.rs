//! Binding telemetry is neither PCM arrival nor persistence. Pin its directory
//! before spawning the helper, and age repeated publications conservatively.
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

const LIMIT: u64 = 16 * 1024;
const SAFE: u64 = 9_007_199_254_740_991;
pub(crate) const FRESH_MS: u64 = 1000;
pub(crate) fn notice_window_visible(visible: Option<bool>, minimized: Option<bool>) -> bool {
    visible == Some(true) && minimized == Some(false)
}
pub(crate) fn notice_target(main_visible: bool, recorder_visible: bool) -> Option<&'static str> {
    if main_visible {
        Some("main")
    } else if recorder_visible {
        Some("recorder")
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Input {
    pub(crate) state: String,
    source_epoch: u64,
    interruptions: u64,
    reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Publication {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    microphone: Input,
    system: Option<Input>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notice {
    id: String,
    target: String,
    kind: String,
    interruptions: u64,
    input: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    report_age_ms: u64,
    microphone: Input,
    system: Option<Input>,
    notices: Vec<Notice>,
}
impl Snapshot {
    pub(crate) fn connected(&self) -> bool {
        self.report_age_ms <= FRESH_MS && self.microphone.state == "connected"
    }
    pub(crate) fn system_connected(&self) -> bool {
        self.report_age_ms <= FRESH_MS
            && self.system.as_ref().is_some_and(|s| s.state == "connected")
    }
}
fn parse(bytes: &[u8], owner: &str) -> Result<Publication, String> {
    if bytes.is_empty() || bytes.len() as u64 > LIMIT || !crate::input_levels::valid_owner(owner) {
        return Err("入力状態の対象または容量が不正です".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let top = value.as_object().ok_or("入力状態の形式が不正です")?;
    let keys = [
        "schemaVersion",
        "ownerId",
        "revision",
        "microphone",
        "system",
    ];
    if top.len() != keys.len() || keys.iter().any(|key| !top.contains_key(*key)) {
        return Err("入力状態の必須項目がありません".into());
    }
    for key in ["microphone", "system"] {
        if key == "system" && value[key].is_null() {
            continue;
        }
        let fields = value[key].as_object().ok_or("入力状態の形式が不正です")?;
        let keys = ["state", "sourceEpoch", "interruptions", "reason"];
        if fields.len() != keys.len() || keys.iter().any(|key| !fields.contains_key(*key)) {
            return Err("入力状態の必須項目がありません".into());
        }
    }
    let p: Publication = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if p.schema_version != 2
        || p.owner_id != owner
        || !(1..=SAFE).contains(&p.revision)
        || !valid_input(&p.microphone, "microphone")
        || p.system.as_ref().is_some_and(|s| !valid_input(s, "system"))
    {
        return Err("入力状態の版または取得状態が不正です".into());
    }
    Ok(p)
}
fn valid_input(m: &Input, input: &str) -> bool {
    let reason_valid = m.reason.as_deref().is_none_or(|reason| {
        if input == "microphone" {
            matches!(
                reason,
                "identity_changed"
                    | "engine_changed"
                    | "device_disconnected"
                    | "identity_ambiguous"
                    | "metadata_unavailable"
                    | "permission_unavailable"
                    | "restart_failed"
                    | "format_changed"
            )
        } else {
            matches!(
                reason,
                "stream_stopped"
                    | "application_unavailable"
                    | "acquisition_timeout"
                    | "permission_unavailable"
                    | "restart_failed"
                    | "format_changed"
            )
        }
    });
    !(m.source_epoch > SAFE
        || m.interruptions > SAFE
        || !reason_valid
        || !matches!(
            m.state.as_str(),
            "connected" | "disconnected" | "unavailable" | "reconnecting" | "stopped"
        )
        || (m.state == "connected"
            && (m.source_epoch == 0 || m.interruptions >= m.source_epoch || m.reason.is_some()))
        || (matches!(m.state.as_str(), "disconnected" | "unavailable")
            && (m.interruptions == 0 || m.reason.is_none())))
}
fn backwards(m: &Input, old: &Input) -> bool {
    m.source_epoch < old.source_epoch
        || m.interruptions < old.interruptions
        || (old.state == "stopped" && m != old)
        || (m.state == "connected"
            && (matches!(old.state.as_str(), "disconnected" | "unavailable")
                || m.interruptions > old.interruptions)
            && m.source_epoch <= old.source_epoch)
}
fn millis(duration: Duration) -> Result<u64, String> {
    u64::try_from(duration.as_millis().saturating_add(1))
        .ok()
        .filter(|v| *v <= SAFE)
        .ok_or_else(|| "入力状態の時計が範囲外です".into())
}
#[derive(Default)]
struct Notices {
    interrupted: u64,
    recovered: u64,
    current: Option<Notice>,
}
impl Notices {
    #[cfg(test)]
    fn observe(&mut self, p: &Publication, age: u64, target: Option<&str>) -> Option<Notice> {
        self.observe_input(p, "microphone", age, target)
    }
    fn observe_input(
        &mut self,
        p: &Publication,
        input: &str,
        age: u64,
        target: Option<&str>,
    ) -> Option<Notice> {
        let m = if input == "microphone" {
            &p.microphone
        } else {
            p.system.as_ref()?
        };
        if age > FRESH_MS || m.state == "stopped" {
            return None;
        }
        let kind = if m.state == "connected" {
            "recovered"
        } else {
            "interrupted"
        };
        let count = if kind == "recovered" {
            &mut self.recovered
        } else {
            &mut self.interrupted
        };
        if m.interruptions > *count {
            if let Some(target) = target.filter(|v| matches!(*v, "main" | "recorder")) {
                *count = m.interruptions;
                self.current = Some(Notice {
                    id: format!("{}:{input}:{}:{kind}", p.owner_id, m.interruptions),
                    target: target.into(),
                    kind: kind.into(),
                    interruptions: m.interruptions,
                    input: input.into(),
                });
            }
        }
        self.current
            .as_ref()
            .filter(|n| n.interruptions == m.interruptions && n.kind == kind)
            .cloned()
    }
}
pub(crate) struct StatusReader {
    directory: File,
    previous: Option<(Publication, u64, Instant, SystemTime)>,
    notices: [Notices; 2],
}
impl StatusReader {
    pub(crate) fn pin(directory: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: crate::input_levels::open_capture_directory(directory)?,
            previous: None,
            notices: Default::default(),
        })
    }
    pub(crate) fn read(&mut self, owner: &str, target: Option<&str>) -> Result<Snapshot, String> {
        let started = Instant::now();
        let file = crate::input_levels::open_capture_file_at(&self.directory, "input-status.json")?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > LIMIT {
            return Err("入力状態のファイルが不正です".into());
        }
        let modified = before.modified().map_err(|e| e.to_string())?;
        let now = SystemTime::now();
        let age = now
            .duration_since(modified)
            .map_err(|_| "入力状態の時計が逆行しました")?;
        let mut bytes = Vec::new();
        (&file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        if before.len() != after.len() || modified != after.modified().map_err(|e| e.to_string())? {
            return Err("入力状態が読込み中に変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.nlink() != 1
                || after.nlink() != 1
                || before.dev() != after.dev()
                || before.ino() != after.ino()
            {
                return Err("入力状態の実体が不正です".into());
            }
        }
        let p = parse(&bytes, owner)?;
        let mut age_ms = millis(age)?.saturating_add(millis(started.elapsed())?);
        if let Some((previous, first_age, first_read, first_wall)) = &self.previous {
            if now < *first_wall {
                return Err("入力状態の時計が逆行しました".into());
            }
            let m = &p.microphone;
            let old = &previous.microphone;
            if p.owner_id != previous.owner_id
                || p.revision < previous.revision
                || backwards(m, old)
                || p.system.is_some() != previous.system.is_some()
                || p.system
                    .as_ref()
                    .zip(previous.system.as_ref())
                    .is_some_and(|(s, old)| backwards(s, old))
            {
                return Err("入力状態の対象・版・取得世代が逆行しました".into());
            }
            if p.revision == previous.revision {
                if p != *previous {
                    return Err("同じ入力状態の版に異なる内容があります".into());
                }
                let elapsed = first_read.elapsed().max(
                    now.duration_since(*first_wall)
                        .map_err(|_| "入力状態の時計が逆行しました")?,
                );
                age_ms = age_ms.max(first_age.saturating_add(millis(elapsed)?));
            } else {
                self.previous = Some((p.clone(), age_ms, Instant::now(), now));
            }
        } else {
            self.previous = Some((p.clone(), age_ms, Instant::now(), now));
        }
        if age_ms > SAFE {
            return Err("入力状態の時計が範囲外です".into());
        }
        let notices = [
            self.notices[0].observe_input(&p, "microphone", age_ms, target),
            self.notices[1].observe_input(&p, "system", age_ms, target),
        ]
        .into_iter()
        .flatten()
        .collect();
        Ok(Snapshot {
            schema_version: 2,
            owner_id: p.owner_id,
            revision: p.revision,
            report_age_ms: age_ms,
            microphone: p.microphone,
            system: p.system,
            notices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::fs;
    const OWNER: &str = "735027d4-a03e-43d3-830a-21e767dfaf88";
    fn value() -> Value {
        json!({"schemaVersion":2,"ownerId":OWNER,"revision":1,"microphone":{"state":"connected","sourceEpoch":1,"interruptions":0,"reason":null},"system":{"state":"connected","sourceEpoch":1,"interruptions":0,"reason":null}})
    }
    fn decoded(v: &Value) -> Publication {
        parse(&serde_json::to_vec(v).unwrap(), OWNER).unwrap()
    }
    fn write(dir: &Path, v: &Value) {
        fs::write(
            dir.join("input-status.json"),
            serde_json::to_vec(v).unwrap(),
        )
        .unwrap();
    }
    fn fixture() -> (tempfile::TempDir, StatusReader) {
        let temp = tempfile::tempdir().unwrap();
        let reader = StatusReader::pin(&temp.path().canonicalize().unwrap()).unwrap();
        write(temp.path(), &value());
        (temp, reader)
    }
    fn interruption(v: &mut Value, count: u64) {
        v["microphone"]["state"] = json!("disconnected");
        v["microphone"]["interruptions"] = json!(count);
        v["microphone"]["reason"] = json!("device_disconnected");
    }
    #[test]
    fn strict_status_requires_owner_safe_counters_known_reason_and_explicit_null() {
        let p = decoded(&value());
        assert_eq!(p.microphone.state, "connected");
        for (field, invalid) in [
            ("schemaVersion", json!(1)),
            ("ownerId", json!("wrong")),
            ("revision", json!(0)),
            ("revision", json!(SAFE + 1)),
            ("extra", json!(true)),
        ] {
            let mut v = value();
            v[field] = invalid;
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        for (field, invalid) in [
            ("sourceEpoch", json!(0)),
            ("sourceEpoch", json!(SAFE + 1)),
            ("interruptions", json!(-1)),
            ("interruptions", json!(1)),
            ("reason", json!("unknown")),
            ("state", json!("saved")),
            ("extra", json!(true)),
        ] {
            let mut v = value();
            v["microphone"][field] = invalid;
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        let mut v = value();
        v["microphone"].as_object_mut().unwrap().remove("reason");
        assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        for state in ["disconnected", "unavailable"] {
            let mut v = value();
            v["microphone"]["state"] = json!(state);
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        assert!(parse(&[], OWNER).is_err());
        assert!(parse(&vec![b' '; LIMIT as usize + 1], OWNER).is_err());
    }
    #[test]
    fn same_publication_does_not_freshen_after_touch_or_atomic_republication() {
        let (temp, mut reader) = fixture();
        reader.read(OWNER, Some("main")).unwrap();
        reader.previous.as_mut().unwrap().1 = 2000;
        write(temp.path(), &value());
        let r = reader.read(OWNER, Some("recorder")).unwrap();
        assert!(!r.connected());
        assert!(r.report_age_ms >= 2000);
        let mut v = value();
        v["revision"] = json!(2);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, Some("main")).unwrap().connected());
    }
    #[test]
    fn owner_revision_and_same_revision_contradictions_are_rejected_without_erasing_floor() {
        let (temp, mut reader) = fixture();
        reader.read(OWNER, None).unwrap();
        let mut v = value();
        interruption(&mut v, 1);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        v["revision"] = json!(2);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_ok());
        write(temp.path(), &value());
        assert!(reader.read(OWNER, None).is_err());
        assert!(reader
            .read("735027d4-a03e-43d3-830a-21e767dfaf89", None)
            .is_err());
    }
    #[test]
    fn recovery_accepts_pending_new_epoch_but_rejects_old_epoch_and_counter_reversal() {
        let (temp, mut reader) = fixture();
        reader.read(OWNER, None).unwrap();
        let mut v = value();
        interruption(&mut v, 1);
        v["revision"] = json!(2);
        write(temp.path(), &v);
        reader.read(OWNER, None).unwrap();
        v["revision"] = json!(3);
        v["microphone"]["state"] = json!("connected");
        v["microphone"]["reason"] = Value::Null;
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        v["microphone"]["state"] = json!("reconnecting");
        v["microphone"]["sourceEpoch"] = json!(2);
        write(temp.path(), &v);
        reader.read(OWNER, None).unwrap();
        v["revision"] = json!(4);
        v["microphone"]["state"] = json!("connected");
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).unwrap().connected());
        v["revision"] = json!(5);
        v["microphone"]["interruptions"] = json!(0);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        v["microphone"]["interruptions"] = json!(1);
        v["microphone"]["sourceEpoch"] = json!(1);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
    }
    #[test]
    fn stopped_policy_and_missing_or_invalid_files_cannot_become_connection_proof() {
        let (temp, mut reader) = fixture();
        let mut v = value();
        v["microphone"]["state"] = json!("stopped");
        write(temp.path(), &v);
        let s = reader.read(OWNER, Some("main")).unwrap();
        assert!(!s.connected());
        assert!(s.notices.is_empty());
        v["revision"] = json!(2);
        v["microphone"]["state"] = json!("connected");
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        fs::remove_file(temp.path().join("input-status.json")).unwrap();
        assert!(reader.read(OWNER, None).is_err());
        fs::write(temp.path().join("input-status.json"), b"not json").unwrap();
        assert!(reader.read(OWNER, None).is_err());
    }
    #[test]
    fn future_files_wall_reversal_and_oversize_are_unknown_not_current_status() {
        let (temp, mut reader) = fixture();
        let path = temp.path().join("input-status.json");
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(60)),
            )
            .unwrap();
        assert!(reader.read(OWNER, None).is_err());
        write(temp.path(), &value());
        reader.read(OWNER, None).unwrap();
        reader.previous.as_mut().unwrap().3 = SystemTime::now() + Duration::from_secs(60);
        let mut v = value();
        v["revision"] = json!(2);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        fs::write(&path, vec![b' '; LIMIT as usize + 1]).unwrap();
        assert!(reader.read(OWNER, None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_leaves_and_symlink_parents_are_rejected() {
        use std::os::unix::fs::symlink;
        let (temp, mut reader) = fixture();
        let leaf = temp.path().join("input-status.json");
        let other = temp.path().join("other.json");
        fs::rename(&leaf, &other).unwrap();
        symlink(&other, &leaf).unwrap();
        assert!(reader.read(OWNER, None).is_err());
        fs::remove_file(&leaf).unwrap();
        fs::hard_link(&other, &leaf).unwrap();
        assert!(reader.read(OWNER, None).is_err());
        let link = temp.path().join("linked");
        symlink(temp.path(), &link).unwrap();
        assert!(StatusReader::pin(&link).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn pinned_directory_never_follows_a_replaced_output_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("output");
        fs::create_dir(&path).unwrap();
        write(&path, &value());
        let mut reader = StatusReader::pin(&path.canonicalize().unwrap()).unwrap();
        let original = temp.path().join("original");
        fs::rename(&path, &original).unwrap();
        fs::create_dir(&path).unwrap();
        let mut v = value();
        interruption(&mut v, 1);
        v["revision"] = json!(2);
        write(&path, &v);
        assert!(reader.read(OWNER, Some("main")).unwrap().connected());
    }
    #[test]
    fn one_notice_destination_is_retained_across_windows_polling_and_reason_changes() {
        let mut notices = Notices::default();
        let mut v = value();
        assert!(notices.observe(&decoded(&v), 0, Some("main")).is_none());
        interruption(&mut v, 1);
        let first = notices.observe(&decoded(&v), 0, Some("main")).unwrap();
        assert_eq!(first.target, "main");
        assert_eq!(first.kind, "interrupted");
        for state in ["unavailable", "reconnecting", "disconnected"] {
            v["microphone"]["state"] = json!(state);
            assert_eq!(
                notices.observe(&decoded(&v), 0, Some("recorder")).unwrap(),
                first
            );
        }
        v["microphone"]["state"] = json!("connected");
        v["microphone"]["reason"] = Value::Null;
        v["microphone"]["sourceEpoch"] = json!(2);
        let recovered = notices.observe(&decoded(&v), 0, Some("recorder")).unwrap();
        assert_eq!(recovered.target, "recorder");
        assert_eq!(recovered.kind, "recovered");
        assert_ne!(first.id, recovered.id);
        assert_eq!(
            notices.observe(&decoded(&v), 0, Some("main")).unwrap(),
            recovered
        );
        interruption(&mut v, 2);
        let next = notices.observe(&decoded(&v), 0, Some("main")).unwrap();
        assert_ne!(first.id, next.id);
    }
    #[test]
    fn stale_stopped_or_no_visible_destination_cannot_publish_a_notice() {
        let mut notices = Notices::default();
        let mut v = value();
        interruption(&mut v, 1);
        assert!(notices.observe(&decoded(&v), 1001, Some("main")).is_none());
        assert!(notices.observe(&decoded(&v), 0, None).is_none());
        assert!(notices.observe(&decoded(&v), 0, Some("settings")).is_none());
        assert_eq!(
            notices
                .observe(&decoded(&v), 0, Some("recorder"))
                .unwrap()
                .target,
            "recorder"
        );
        v["microphone"]["state"] = json!("stopped");
        assert!(notices.observe(&decoded(&v), 0, Some("main")).is_none());
    }
    #[test]
    fn main_visibility_has_priority_and_hidden_windows_do_not_receive_new_notices() {
        assert_eq!(notice_target(true, true), Some("main"));
        assert_eq!(notice_target(true, false), Some("main"));
        assert_eq!(notice_target(false, true), Some("recorder"));
        assert_eq!(notice_target(false, false), None);
    }
    #[test]
    fn minimized_or_unconfirmed_window_state_cannot_receive_new_notices() {
        assert!(notice_window_visible(Some(true), Some(false)));
        for (visible, minimized) in [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (Some(false), Some(true)),
            (None, Some(false)),
            (Some(true), None),
            (None, None),
        ] {
            assert!(!notice_window_visible(visible, minimized));
            assert_eq!(
                notice_target(notice_window_visible(visible, minimized), true),
                Some("recorder")
            );
        }
        assert_eq!(
            notice_target(false, notice_window_visible(Some(true), Some(true))),
            None
        );
    }
    fn system_interruption(v: &mut Value, count: u64) {
        v["system"]["state"] = json!("disconnected");
        v["system"]["interruptions"] = json!(count);
        v["system"]["reason"] = json!("stream_stopped");
    }
    #[test]
    fn system_binding_requires_schema_two_explicit_availability_and_lane_specific_reasons() {
        for field in ["state", "sourceEpoch", "interruptions", "reason"] {
            let mut v = value();
            v["system"].as_object_mut().unwrap().remove(field);
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        let mut v = value();
        v.as_object_mut().unwrap().remove("system");
        assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        v = value();
        v["system"] = Value::Null;
        assert!(decoded(&v).system.is_none());
        for (field, bad) in [
            ("sourceEpoch", json!(0)),
            ("sourceEpoch", json!(SAFE + 1)),
            ("interruptions", json!(-1)),
            ("interruptions", json!(1)),
            ("reason", json!("unknown")),
            ("state", json!("saved")),
            ("extra", json!(true)),
        ] {
            let mut v = value();
            v["system"][field] = bad;
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        v = value();
        system_interruption(&mut v, 1);
        for reason in [
            "stream_stopped",
            "application_unavailable",
            "acquisition_timeout",
            "permission_unavailable",
            "restart_failed",
            "format_changed",
        ] {
            v["system"]["reason"] = json!(reason);
            decoded(&v);
        }
        for reason in [
            "device_disconnected",
            "identity_changed",
            "metadata_unavailable",
        ] {
            v["system"]["reason"] = json!(reason);
            assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
        }
        interruption(&mut v, 1);
        v["microphone"]["reason"] = json!("stream_stopped");
        assert!(parse(&serde_json::to_vec(&v).unwrap(), OWNER).is_err());
    }
    #[test]
    fn unsupported_system_is_not_connected_and_cannot_change_backend_within_one_owner() {
        for unsupported_first in [true, false] {
            let (temp, mut reader) = fixture();
            let mut v = value();
            if unsupported_first {
                v["system"] = Value::Null;
            }
            write(temp.path(), &v);
            let r = reader.read(OWNER, Some("main")).unwrap();
            assert!(r.connected());
            assert_eq!(r.system_connected(), !unsupported_first);
            assert!(r.notices.is_empty());
            v["revision"] = json!(2);
            v["system"] = if unsupported_first {
                value()["system"].clone()
            } else {
                Value::Null
            };
            write(temp.path(), &v);
            assert!(reader.read(OWNER, Some("main")).is_err());
        }
    }
    #[test]
    fn system_changes_are_independent_monotonic_and_stop_is_sticky() {
        let (temp, mut reader) = fixture();
        reader.read(OWNER, None).unwrap();
        let mut v = value();
        system_interruption(&mut v, 1);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err()); // immutable publication
        v["revision"] = json!(2);
        write(temp.path(), &v);
        let r = reader.read(OWNER, Some("main")).unwrap();
        assert!(r.connected());
        assert!(!r.system_connected());
        assert_eq!(r.notices.len(), 1);
        assert_eq!(r.notices[0].input, "system");
        v["revision"] = json!(3);
        v["system"]["state"] = json!("connected");
        v["system"]["reason"] = Value::Null;
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err()); // invalid old epoch
        v["system"]["state"] = json!("reconnecting");
        v["system"]["sourceEpoch"] = json!(2);
        write(temp.path(), &v);
        reader.read(OWNER, None).unwrap();
        v["revision"] = json!(4);
        v["system"]["state"] = json!("connected");
        write(temp.path(), &v);
        let r = reader.read(OWNER, Some("recorder")).unwrap();
        assert!(r.connected());
        assert!(r.system_connected());
        assert_eq!(r.notices[0].kind, "recovered");
        assert_eq!(r.notices[0].target, "recorder");
        v["revision"] = json!(5);
        v["system"]["interruptions"] = json!(0);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        v["system"]["interruptions"] = json!(1);
        v["system"]["sourceEpoch"] = json!(1);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
        v["system"]["sourceEpoch"] = json!(2);
        v["system"]["state"] = json!("stopped");
        write(temp.path(), &v);
        assert!(reader.read(OWNER, Some("main")).unwrap().notices.is_empty());
        v["revision"] = json!(6);
        v["system"]["state"] = json!("connected");
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).is_err());
    }
    #[test]
    fn simultaneous_notices_keep_separate_input_ids_fixed_destinations_and_real_age() {
        let (temp, mut reader) = fixture();
        let mut v = value();
        interruption(&mut v, 1);
        system_interruption(&mut v, 1);
        write(temp.path(), &v);
        assert!(reader.read(OWNER, None).unwrap().notices.is_empty());
        let r = reader.read(OWNER, Some("main")).unwrap();
        assert_eq!(r.notices.len(), 2);
        assert_ne!(r.notices[0].id, r.notices[1].id);
        assert_eq!(r.notices[0].input, "microphone");
        assert_eq!(r.notices[1].input, "system");
        assert!(reader
            .read(OWNER, Some("recorder"))
            .unwrap()
            .notices
            .iter()
            .all(|n| n.target == "main"));
        v["revision"] = json!(2);
        v["system"]["sourceEpoch"] = json!(2);
        v["system"]["state"] = json!("connected");
        v["system"]["reason"] = Value::Null;
        write(temp.path(), &v);
        let r = reader.read(OWNER, Some("recorder")).unwrap();
        assert_eq!(r.notices[0].target, "main");
        assert_eq!(r.notices[0].kind, "interrupted");
        assert_eq!(r.notices[1].target, "recorder");
        assert_eq!(r.notices[1].kind, "recovered");
        reader.previous.as_mut().unwrap().1 = 2000;
        write(temp.path(), &v);
        let stale = reader.read(OWNER, Some("main")).unwrap();
        assert!(!stale.connected());
        assert!(!stale.system_connected());
        assert!(stale.notices.is_empty());
    }
    #[test]
    #[ignore = "Swift-owned fixture; run make check-input-status without audio capture"]
    fn swift_generated_input_status_is_owner_bound_and_never_pcm_proof() {
        let path = std::path::PathBuf::from(
            std::env::var("MON_INPUT_STATUS_FIXTURE_DIR").expect("Swift fixture directory"),
        );
        let before = fs::read(path.join("input-status.json")).unwrap();
        let mut reader = StatusReader::pin(&path.canonicalize().unwrap()).unwrap();
        let r = reader.read(OWNER, Some("main")).unwrap();
        assert_eq!(r.revision, 4);
        assert_eq!(r.microphone.source_epoch, 2);
        assert_eq!(r.microphone.interruptions, 1);
        let system = r.system.as_ref().unwrap();
        assert_eq!(system.source_epoch, 2);
        assert_eq!(system.interruptions, 1);
        // A standalone run may compile Rust after Swift writes the fixture.
        // Keep its real age: never touch it to manufacture a fresh heartbeat.
        let fresh = r.report_age_ms <= FRESH_MS;
        assert_eq!(r.connected(), fresh);
        if fresh {
            assert_eq!(r.notices.len(), 2);
            assert!(r.notices.iter().all(|n| n.kind == "recovered"));
            assert!(r.system_connected());
        } else {
            assert!(r.notices.is_empty());
            assert!(!r.system_connected());
        }
        let other = reader.read(OWNER, Some("recorder")).unwrap();
        for notice in other.notices {
            assert_eq!(notice.target, "main");
        }
        assert_eq!(before, fs::read(path.join("input-status.json")).unwrap());
        assert!(!path.join("microphone.caf").exists());
        assert!(!path.join("system.caf").exists());
    }
}
