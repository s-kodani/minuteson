//! Binding telemetry is neither PCM arrival nor persistence. Pin its directory
//! before spawning the helper, and age repeated publications conservatively.
mod clock;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

pub(crate) mod archived;
pub(crate) mod diagnostics;
mod history;
#[cfg(test)]
mod history_tests;
pub(crate) mod journal;
pub(crate) mod persistence;
pub(crate) mod queue;
use history::History;
const LIMIT: u64 = 64 * 1024;
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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Publication {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    microphone: Input,
    system: Option<Input>,
    #[serde(default)]
    microphone_history: Option<History>,
    #[serde(default)]
    system_history: Option<History>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notice {
    id: String,
    target: String,
    kind: String,
    interruptions: u64,
    input: String,
    recipient_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: Option<queue::Delivery>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Receipt {
    schema_version: u8,
    owner_id: String,
    notice_id: String,
    target: String,
    recipient_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: Option<queue::Delivery>,
}
impl Receipt {
    pub(crate) fn account_wait(&mut self, elapsed: std::time::Duration) {
        if let Some(delivery) = &mut self.delivery {
            delivery.account_persistence_time(elapsed);
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    notice_loss: Option<queue::Loss>,
    #[serde(skip)]
    observed_at: Instant,
    #[serde(skip)]
    observed_wall: SystemTime,
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
    let mut keys = vec![
        "schemaVersion",
        "ownerId",
        "revision",
        "microphone",
        "system",
    ];
    keys.extend(["microphoneHistory", "systemHistory"]);
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
    if p.schema_version != 3
        || p.owner_id != owner
        || !(1..=SAFE).contains(&p.revision)
        || !valid_input(&p.microphone, "microphone")
        || p.system.as_ref().is_some_and(|s| !valid_input(s, "system"))
    {
        return Err("入力状態の版または取得状態が不正です".into());
    }
    if !p.microphone_history.as_ref().is_some_and(|h| {
        history::closed(&value["microphoneHistory"]) && h.valid(&p.microphone, "microphone")
    }) || p.system.is_some() != p.system_history.is_some()
        || p.system
            .as_ref()
            .zip(p.system_history.as_ref())
            .is_some_and(|(s, h)| {
                !history::closed(&value["systemHistory"]) || !h.valid(s, "system")
            })
    {
        return Err("入力状態の遷移履歴が不正です".into());
    }
    Ok(p)
}
fn valid_input(m: &Input, input: &str) -> bool {
    let reason_valid = m.reason.as_deref().is_none_or(|reason| {
        if input == "microphone" {
            matches!(
                reason,
                "identity_changed"
                    | "device_changed"
                    | "engine_changed"
                    | "device_disconnected"
                    | "identity_ambiguous"
                    | "metadata_unavailable"
                    | "permission_unavailable"
                    | "restart_failed"
                    | "format_changed"
                    | "clock_changed"
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
                    | "clock_changed"
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
pub(crate) struct StatusReader {
    clock: clock::Clock,
    directory: File,
    previous: Option<(Publication, u64, Instant, SystemTime)>,
    queue: queue::Queue,
    ledger: Option<queue::Ledger>,
    persistence: persistence::Reader,
}
impl StatusReader {
    pub(crate) fn pin(directory: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: crate::input_levels::open_capture_directory(directory)?,
            clock: clock::Clock::default(),
            previous: None,
            queue: Default::default(),
            ledger: None,
            persistence: Default::default(),
        })
    }
    pub(crate) fn persistence(&mut self, owner: &str) -> Result<persistence::Snapshot, String> {
        self.persistence.read(&self.directory, owner)
    }
    pub(crate) fn read(&mut self, owner: &str, target: Option<&str>) -> Result<Snapshot, String> {
        let started = self.clock.instant();
        let file = crate::input_levels::open_capture_file_at(&self.directory, "input-status.json")?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > LIMIT {
            return Err("入力状態のファイルが不正です".into());
        }
        let modified = before.modified().map_err(|e| e.to_string())?;
        let now = self.clock.wall();
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
        let mut age_ms = millis(age)?.saturating_add(millis(self.clock.elapsed(started))?);
        if let Some((previous, first_age, first_read, first_wall)) = &self.previous {
            if now < *first_wall {
                return Err("入力状態の時計が逆行しました".into());
            }
            let m = &p.microphone;
            let old = &previous.microphone;
            if p.owner_id != previous.owner_id
                || p.schema_version != previous.schema_version
                || p.revision < previous.revision
                || backwards(m, old)
                || p.system.is_some() != previous.system.is_some()
                || p.system
                    .as_ref()
                    .zip(previous.system.as_ref())
                    .is_some_and(|(s, old)| backwards(s, old))
                || p.microphone_history
                    .as_ref()
                    .zip(previous.microphone_history.as_ref())
                    .is_some_and(|(h, old)| !h.advances(old))
                || p.system_history
                    .as_ref()
                    .zip(previous.system_history.as_ref())
                    .is_some_and(|(h, old)| !h.advances(old))
            {
                return Err("入力状態の対象・版・取得世代が逆行しました".into());
            }
            if p.revision == previous.revision {
                if p != *previous {
                    return Err("同じ入力状態の版に異なる内容があります".into());
                }
                let elapsed = self.clock.elapsed(*first_read).max(
                    now.duration_since(*first_wall)
                        .map_err(|_| "入力状態の時計が逆行しました")?,
                );
                age_ms = age_ms.max(first_age.saturating_add(millis(elapsed)?));
            } else {
                self.previous = Some((p.clone(), age_ms, self.clock.instant(), now));
            }
        } else {
            self.previous = Some((p.clone(), age_ms, self.clock.instant(), now));
        }
        if age_ms > SAFE {
            return Err("入力状態の時計が範囲外です".into());
        }
        let queue_started = self.clock.instant();
        if self.ledger.is_none() {
            let (mut ledger, restored) = queue::Ledger::open(&self.directory, owner)?;
            if let Some((prior, mut queue)) = restored {
                queue::Ledger::advances(&p, &prior)?;
                ledger.fence(&mut queue)?;
                self.queue = queue;
            }
            self.ledger = Some(ledger);
        }
        let mut candidate = self.queue.clone();
        let mut notices = candidate.observe(&p, age_ms, target, self.clock.instant());
        self.ledger.as_mut().unwrap().commit(&p, &candidate)?;
        self.queue = candidate;
        // File sync is not a new producer heartbeat. Account for time spent
        // persisting before any connection flag or delivery offer leaves Rust.
        age_ms = age_ms.saturating_add(millis(self.clock.elapsed(queue_started))?);
        if age_ms > SAFE {
            return Err("入力状態の時計が範囲外です".into());
        }
        if age_ms > FRESH_MS {
            notices.clear();
        }
        let notice_loss = self.queue.loss();
        notice_loss.log_if_incomplete("live-notice-loss", owner);
        Ok(Snapshot {
            schema_version: 4,
            owner_id: p.owner_id,
            revision: p.revision,
            report_age_ms: age_ms,
            microphone: p.microphone,
            system: p.system,
            notices,
            notice_loss: Some(notice_loss),
            observed_at: self.clock.instant(),
            observed_wall: self.clock.wall(),
        })
    }
    // Receipt is owned by the live recorder, not the disposable WebView. It is
    // neither a persistence receipt for audio nor evidence of actual rendering.
    pub(crate) fn queued_action(
        &mut self,
        snapshot: &Snapshot,
        notice_id: &str,
        recipient: &str,
        target: &str,
        visible: bool,
        generation: u64,
        action: queue::Action,
        receive_id: Option<&str>,
    ) -> Result<Receipt, String> {
        let p = &self.previous.as_ref().ok_or("入力報告がありません")?.0;
        let elapsed = self.clock.elapsed(snapshot.observed_at).max(
            self.clock
                .wall()
                .duration_since(snapshot.observed_wall)
                .map_err(|_| "通知の時計が逆行しました")?,
        );
        if snapshot.schema_version != 4
            || snapshot.owner_id != p.owner_id
            || snapshot.revision != p.revision
            || snapshot.microphone != p.microphone
            || snapshot.system != p.system
            || snapshot
                .report_age_ms
                .saturating_add(elapsed.as_millis().try_into().unwrap_or(u64::MAX))
                > FRESH_MS
            || !visible
            || !matches!(target, "main" | "recorder")
            || !crate::input_levels::valid_owner(recipient)
            || notice_id.len() > 128
            || !(1..=SAFE).contains(&generation)
        {
            return Err("通知の録音・報告・表示状態または受付世代が不正です".into());
        }
        let mut candidate = self.queue.clone();
        let action_started = self.clock.instant();
        let mut receipt = candidate.act(
            &p.owner_id,
            notice_id,
            recipient,
            target,
            generation,
            action,
            action_started,
            receive_id,
        )?;
        self.ledger
            .as_mut()
            .ok_or("通知の永続台帳がありません")?
            .commit(p, &candidate)?;
        self.queue = candidate;
        if let Some(delivery) = &mut receipt.delivery {
            delivery.account_persistence_time(self.clock.elapsed(action_started));
        }
        Ok(receipt)
    }
}

#[cfg(test)]
#[path = "input_status/current_tests.rs"]
mod tests;
