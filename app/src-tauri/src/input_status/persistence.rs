//! Journal writer diagnostics have their own heartbeat and revision domain.
//! Neither current input acquisition nor saved audio follows from this result.
use super::{millis, SAFE};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    time::{Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Persistence {
    requested_revision: u64,
    committed_revision: u64,
    state: State,
    closed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum State {
    Confirmed,
    Pending,
    Failed,
    Invalid,
}
impl Persistence {
    fn valid(&self) -> bool {
        (1..=SAFE).contains(&self.requested_revision)
            && (1..=self.requested_revision).contains(&self.committed_revision)
            && (self.state != State::Confirmed
                || self.requested_revision == self.committed_revision)
    }
    fn advances(&self, prior: &Self) -> bool {
        self.requested_revision >= prior.requested_revision
            && self.committed_revision >= prior.committed_revision
            && (!prior.closed || self.closed)
            && (prior.state != State::Invalid || self.state == State::Invalid)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Report {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    persistence: Persistence,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    report_age_ms: u64,
    persistence: Persistence,
}

/// Reopen the leaf against the SAME pinned directory after reading; accepting
/// a replaced ancestor, hard link, foreign uid or changed bytes is not a result.
fn bytes(
    directory: &File,
    name: &str,
    limit: u64,
) -> Result<Option<(Vec<u8>, SystemTime)>, String> {
    let Some(file) = crate::input_levels::open_capture_file_at_optional(directory, name)? else {
        return Ok(None);
    };
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() == 0 || before.len() > limit {
        return Err("入力履歴診断の容量が不正です".into());
    }
    let mut data = Vec::new();
    (&file)
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let read_end = file.metadata().map_err(|e| e.to_string())?;
    let after = crate::input_levels::open_capture_file_at(directory, name)?
        .metadata()
        .map_err(|e| e.to_string())?;
    let modified = before.modified().map_err(|e| e.to_string())?;
    for m in [&read_end, &after] {
        if data.len() as u64 != before.len()
            || before.len() != m.len()
            || modified != m.modified().map_err(|e| e.to_string())?
        {
            return Err("入力履歴診断が読込み中に変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.uid() != unsafe { libc::geteuid() }
                || m.uid() != before.uid()
                || before.nlink() != 1
                || m.nlink() != 1
                || before.dev() != m.dev()
                || before.ino() != m.ino()
                || before.ctime() != m.ctime()
                || before.ctime_nsec() != m.ctime_nsec()
            {
                return Err("入力履歴診断の実体または所有が不正です".into());
            }
        }
    }
    Ok(Some((data, modified)))
}
fn parse(data: &[u8], owner: &str) -> Result<Report, String> {
    let r: Report = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    if data.is_empty()
        || data.len() > 4096
        || !crate::input_levels::valid_owner(owner)
        || r.owner_id != owner
        || r.schema_version != 1
        || !(1..=SAFE).contains(&r.revision)
        || !r.persistence.valid()
    {
        return Err("入力履歴診断の所有・版または保存結果が不正です".into());
    }
    Ok(r)
}
#[derive(Default)]
pub(super) struct Reader {
    previous: Option<(Report, u64, Instant, SystemTime)>,
}
impl Reader {
    pub(super) fn read(&mut self, directory: &File, owner: &str) -> Result<Snapshot, String> {
        let started = Instant::now();
        let (data, modified) = bytes(directory, "input-journal-status.json", 4096)?
            .ok_or("入力履歴診断の報告がありません")?;
        let now = SystemTime::now();
        let report = parse(&data, owner)?;
        let mut age = millis(
            now.duration_since(modified)
                .map_err(|_| "入力履歴診断の時計が逆行しました")?,
        )?
        .saturating_add(millis(started.elapsed())?);
        if let Some((prior, first_age, at, wall)) = &self.previous {
            if now < *wall
                || report.owner_id != prior.owner_id
                || report.revision < prior.revision
                || !report.persistence.advances(&prior.persistence)
            {
                return Err("入力履歴診断の版・保存結果または時計が逆行しました".into());
            }
            if report.revision == prior.revision {
                if report != *prior {
                    return Err("同じ入力履歴診断の版が変更されました".into());
                }
                age = age.max(
                    first_age.saturating_add(millis(
                        at.elapsed().max(
                            now.duration_since(*wall)
                                .map_err(|_| "入力履歴診断の時計が逆行しました")?,
                        ),
                    )?),
                );
            } else {
                self.previous = Some((report.clone(), age, Instant::now(), now));
            }
        } else {
            self.previous = Some((report.clone(), age, Instant::now(), now));
        }
        if age > SAFE {
            return Err("入力履歴診断の年齢が範囲外です".into());
        }
        Ok(Snapshot {
            schema_version: 1,
            owner_id: report.owner_id,
            revision: report.revision,
            report_age_ms: age,
            persistence: report.persistence,
        })
    }
}

/// The final manifest carries a structured copy, not a new commit receipt.
/// A bounded finish timeout remains unconfirmed even if the writer completes
/// immediately afterwards. No diagnostics in a legacy manifest means unknown,
/// never 'confirmed'. Do not display arbitrary error strings or paths.
pub(super) fn archived_issue(directory: &File, owner: &str) -> Option<&'static str> {
    final_issue(directory, owner).unwrap_or(Some("unavailable"))
}
fn final_issue(directory: &File, owner: &str) -> Result<Option<&'static str>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Final {
        #[serde(default)]
        recording_owner_id: Option<String>,
        #[serde(default)]
        input_event_journal_error: Option<String>,
        #[serde(default)]
        input_event_journal_persistence: Option<Persistence>,
        state: String,
    }
    let Some((data, _)) = bytes(directory, "manifest.json", 256 * 1024)? else {
        return retained_issue(directory, owner);
    };
    let final_report: Final = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    if final_report.input_event_journal_persistence.is_none()
        && final_report.input_event_journal_error.is_none()
    {
        return retained_issue(directory, owner);
    }
    if final_report.recording_owner_id.as_deref() != Some(owner)
        || !crate::input_levels::valid_owner(owner)
        || !matches!(final_report.state.as_str(), "saved" | "failed")
    {
        return Err("停止時の履歴診断の所有または状態が不正です".into());
    }
    if let Some(p) = final_report.input_event_journal_persistence {
        if !p.valid() || !p.closed {
            return Err("停止時の履歴診断が不正です".into());
        }
        match p.state {
            State::Invalid => return Ok(Some("invalid")),
            State::Failed => return Ok(Some("failed")),
            State::Pending => return Ok(Some("unconfirmed")),
            State::Confirmed => {
                if final_report.input_event_journal_error.is_none() {
                    let retained = super::journal::Reader::pin_at(directory)?.read_record(owner)?;
                    if !retained.sealed || retained.report.revision != p.committed_revision {
                        return Err("停止時の履歴保存診断と保存済み履歴が一致しません".into());
                    }
                }
            }
        }
    }
    Ok(final_report
        .input_event_journal_error
        .map(|_| "unconfirmed"))
}

fn retained_issue(directory: &File, owner: &str) -> Result<Option<&'static str>, String> {
    if crate::input_levels::open_capture_file_at_optional(directory, "input-events.json")?.is_none()
    {
        return Ok(None); // pre-journal recording: unknown, no fabricated result
    }
    let record = super::journal::Reader::pin_at(directory)?.read_record(owner)?;
    Ok((!record.sealed).then_some("unconfirmed"))
}

#[cfg(all(test, unix))]
mod tests;
