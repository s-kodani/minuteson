//! Retained PRODUCER evidence, intentionally not a live Snapshot or a delivery
//! receipt. File age cannot freshen binding telemetry. `sealed` only closes the
//! producer journal; neither PCM durability nor rendering follows from it.
use super::{history::History, Publication};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const LIMIT: u64 = 65 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Record {
    pub(super) schema_version: u8,
    pub(super) sealed: bool,
    pub(super) report: Publication,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Retained {
    schema_version: u8,
    owner_id: String,
    revision: u64,
    sealed: bool,
    microphone_history: History,
    system_history: Option<History>,
}

pub(super) fn parse(bytes: &[u8], owner: &str) -> Result<Record, String> {
    if bytes.is_empty() || bytes.len() as u64 > LIMIT {
        return Err("録音入力の保存履歴が容量範囲外です".into());
    }
    // Decode the original bytes too: Value alone would erase duplicate keys.
    let record: Record = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let report = super::parse(
        &serde_json::to_vec(&value["report"]).map_err(|e| e.to_string())?,
        owner,
    )?;
    if record.schema_version != 1
        || report.schema_version != 3
        || report != record.report
        || (record.sealed
            && (report.microphone.state != "stopped"
                || report.system.as_ref().is_some_and(|s| s.state != "stopped")))
    {
        return Err("録音入力の保存履歴の版または封印状態が不正です".into());
    }
    Ok(record)
}

pub(crate) struct Reader {
    directory: File,
    previous: Option<Record>,
}

impl Reader {
    pub(crate) fn pin(directory: &Path) -> Result<Self, String> {
        Ok(Self {
            directory: crate::input_levels::open_capture_directory(directory)?,
            previous: None,
        })
    }

    pub(super) fn pin_at(directory: &File) -> Result<Self, String> {
        Ok(Self {
            directory: directory.try_clone().map_err(|e| e.to_string())?,
            previous: None,
        })
    }

    pub(crate) fn read(&mut self, owner: &str) -> Result<Retained, String> {
        let record = self.read_record(owner)?;
        Ok(Retained {
            schema_version: 1,
            owner_id: record.report.owner_id,
            revision: record.report.revision,
            sealed: record.sealed,
            microphone_history: record.report.microphone_history.unwrap(),
            system_history: record.report.system_history,
        })
    }
    pub(super) fn read_record(&mut self, owner: &str) -> Result<Record, String> {
        let file = crate::input_levels::open_capture_file_at(&self.directory, "input-events.json")?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > LIMIT {
            return Err("録音入力の保存履歴のファイルが不正です".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.nlink() != 1
                || before.uid() != unsafe { libc::geteuid() }
                || before.mode() & 0o077 != 0
            {
                return Err("録音入力の保存履歴の所有・リンク・権限が不正です".into());
            }
        }
        let mut bytes = Vec::new();
        (&file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let read_end = file.metadata().map_err(|e| e.to_string())?;
        let after =
            crate::input_levels::open_capture_file_at(&self.directory, "input-events.json")?
                .metadata()
                .map_err(|e| e.to_string())?;
        if bytes.len() as u64 != before.len()
            || before.len() != read_end.len()
            || before.modified().map_err(|e| e.to_string())?
                != read_end.modified().map_err(|e| e.to_string())?
            || before.len() != after.len()
            || before.modified().map_err(|e| e.to_string())?
                != after.modified().map_err(|e| e.to_string())?
        {
            return Err("録音入力の保存履歴が読込み中に変更されました".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.dev() != after.dev()
                || before.ino() != after.ino()
                || before.nlink() != after.nlink()
                || before.nlink() != read_end.nlink()
                || before.ctime() != read_end.ctime()
                || before.ctime_nsec() != read_end.ctime_nsec()
                || before.ctime() != after.ctime()
                || before.ctime_nsec() != after.ctime_nsec()
            {
                return Err("録音入力の保存履歴の実体が変更されました".into());
            }
        }
        let record = parse(&bytes, owner)?;
        if let Some(old) = &self.previous {
            let p = &record.report;
            let prior = &old.report;
            if old.sealed && record != *old
                || p.owner_id != prior.owner_id
                || p.revision < prior.revision
                || (p.revision == prior.revision && record != *old)
                || super::backwards(&p.microphone, &prior.microphone)
                || p.system.is_some() != prior.system.is_some()
                || p.system
                    .as_ref()
                    .zip(prior.system.as_ref())
                    .is_some_and(|(s, old)| super::backwards(s, old))
                || p.microphone_history
                    .as_ref()
                    .zip(prior.microphone_history.as_ref())
                    .is_some_and(|(h, old)| !h.advances(old))
                || p.system_history
                    .as_ref()
                    .zip(prior.system_history.as_ref())
                    .is_some_and(|(h, old)| !h.advances(old))
            {
                return Err("録音入力の保存履歴の版・取得世代・封印が逆行しました".into());
            }
        }
        self.previous = Some(record.clone());
        Ok(record)
    }
}

#[cfg(test)]
mod tests;
