//! Native publication input derived from a sealed, typed historical restore.
//! No portable JSON, consent, deletion path list or native source receipt is
//! accepted as a capability. This is not a file-removal permission.
pub(super) fn operation_key(archive: &str, meeting: &str) -> String {
    format!("restored-audio-unavailable:{archive}:{meeting}")
}

#[cfg(any(unix, windows))]
use super::{
    archive, check_cancelled, ledger, restore_fixed::workspace::seal::SealedWorkspace,
    restore_ids::RestoreIds,
};
#[cfg(any(unix, windows))]
use crate::storage::deletion::audio::restored::History;

#[cfg(any(unix, windows))]
pub(in crate::storage) struct Meeting {
    pub(in crate::storage) source: String,
    pub(in crate::storage) target: String,
    pub(in crate::storage) operation: String,
    pub(in crate::storage) history: History,
}

#[cfg(any(unix, windows))]
pub(in crate::storage) struct Seed {
    batch: String,
    archive: String,
    fingerprint: String,
    meetings: Vec<Meeting>,
}

#[cfg(any(unix, windows))]
impl Seed {
    pub(in crate::storage::backup) fn prepare(
        sealed: &SealedWorkspace,
        ids: &RestoreIds<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        sealed.matches_ids(ids, check)?;
        sealed.verify(check)?;
        let source = ids.source();
        source.verify_validation_database(check)?;
        source
            .deleted_audio
            .verify_omissions(&source.manifest.omitted_assets)?;
        let mut meetings = Vec::new();
        for source_meeting in source.deleted_audio.meetings() {
            check_cancelled(check)?;
            source
                .deleted_audio
                .verify_meeting(&source.database, source_meeting)?;
            let target = ids.get("meetings", "", source_meeting)?;
            let mut assets = std::collections::BTreeSet::new();
            for asset in &source.manifest.omitted_assets {
                if asset.meeting_id != source_meeting
                    || asset.reason != archive::OmittedReason::SourceAudioDeleted
                {
                    continue;
                }
                let id = ids.get("file_assets", source_meeting, &asset.asset_id)?;
                let matches: bool = sealed.connection().query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_assets WHERE id=?1 AND meeting_id=?2 AND relative_path=?3 AND media_type LIKE 'audio/%' AND state='missing' AND size_bytes=?4 AND content_hash=?5)",
                    rusqlite::params![id, target, format!("meetings/{target}/restored/{id}"), i64::try_from(asset.source_size).map_err(|_| "音声容量が範囲外です")?, asset.source_sha256],
                    |row| row.get(0),
                ).map_err(|e| e.to_string())?;
                if !matches || !assets.insert(id) {
                    return Err("封印した音声欠落と検査済み資産が一致しません".into());
                }
            }
            let history = History::capture(sealed.connection(), &target, check)?;
            if history.asset_ids() != assets {
                return Err("封印した全音声資産が欠落履歴にありません".into());
            }
            meetings.push(Meeting {
                source: source_meeting.into(),
                operation: ids.get(
                    "operation",
                    source_meeting,
                    &operation_key(&ids.archive_id, source_meeting),
                )?,
                target,
                history,
            });
        }
        check_cancelled(check)?;
        Ok(Self {
            batch: ids.batch_id.clone(),
            archive: ids.archive_id.clone(),
            fingerprint: ledger::fingerprint(source)?,
            meetings,
        })
    }
    pub(in crate::storage) fn batch(&self) -> &str {
        &self.batch
    }
    pub(in crate::storage) fn archive(&self) -> &str {
        &self.archive
    }
    pub(in crate::storage) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub(in crate::storage) fn meetings(&self) -> &[Meeting] {
        &self.meetings
    }
}
