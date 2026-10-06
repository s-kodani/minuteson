//! Audio-only review, not a whole-meeting deletion permit. Inspection never
//! changes rows or removes files. Consent and durable removal follow separately.
use super::{
    database::Scope,
    files::{Census as MeetingCensus, Entry as FileEntry, Manifest, MAX_ENTRIES},
};
use crate::storage::Storage;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Track {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub chunks: u64,
    pub duration_us: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Disposition {
    RegisteredAudio,
    ProducerAudioLeftover,
    AnalysisPcm,
    Retain,
    Unclassified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub relative_path: String,
    pub asset_id: Option<String>,
    pub disposition: Disposition,
    pub present: bool,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Retained {
    pub analysis_revisions: u64,
    pub utterances: u64,
    pub minutes_revisions: u64,
    pub drafts: u64,
    pub edit_operations: u64,
    pub evidence_links: u64,
    pub jobs: u64,
    pub external_export_records: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub schema_version: u32,
    pub inspection_id: String,
    pub meeting_id: String,
    pub census_hash: String,
    pub tracks: Vec<Track>,
    pub retained: Retained,
    pub entries: Vec<Entry>,
    pub audio_files: usize,
    pub audio_bytes: u64,
    pub missing_audio_files: usize,
    pub changed_audio_files: usize,
    pub unregistered_audio_files: usize,
    pub unclassified_files: usize,
    // Serialized views and producer-name hints are never deletion authority.
    pub execute_allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Records {
    tracks: Vec<Track>,
    audio_assets: BTreeMap<String, String>,
}

#[path = "audio_intent.rs"]
pub(crate) mod intent;

pub(in crate::storage) fn is_audio_media(media: &str) -> bool {
    media
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("audio/"))
}

// Audio history remains readable, but cannot become an input again. This
// validates typed completion evidence instead of trusting mutable ready flags.
pub(in crate::storage) fn require_source(db: &Connection, meeting: &str) -> Result<(), String> {
    if intent::removal::completion::deleted(db, meeting)? {
        return Err("音声は削除済みのため再生・再解析できません。文字起こし・議事録・根拠は保持されています".into());
    }
    Ok(())
}

// This saved observation is not a file-removal capability. No external path,
// transcript body, document payload or whole-meeting Prepared is serialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    scope: Scope,
    manifest: Manifest,
    records: Records,
}
impl Snapshot {
    fn validate(&self, db: &Connection) -> Result<(), String> {
        super::intent::validate_observations(db, &self.scope, &self.manifest)?;
        if self.records.tracks.len() > MAX_ENTRIES
            || self.records.audio_assets.len() > MAX_ENTRIES
            || self.records.tracks.windows(2).any(|w| w[0].id >= w[1].id)
            || self.records.tracks.iter().any(|track| {
                !uuid(&track.id)
                    || !matches!(
                        track.kind.as_str(),
                        "mic" | "system" | "imported" | "processed"
                    )
                    || !matches!(track.state.as_str(), "ready" | "interrupted" | "failed")
                    || track.chunks > MAX_ENTRIES as u64
                    || track.duration_us < 0
            })
            || self.records.audio_assets.iter().any(|(path, id)| {
                !uuid(id) || !super::files::valid_path(path, &self.scope.meeting_id)
            })
        {
            return Err("音声だけの削除の保存されたトラック・資産が不正です".into());
        }
        let mut parents = BTreeMap::new();
        let root = format!("meetings/{}", self.scope.meeting_id);
        for entry in &self.manifest.entries {
            if entry.proof.is_some() && entry.path != root {
                let parent = entry
                    .path
                    .rsplit_once('/')
                    .ok_or("音声観測の親がありません")?
                    .0;
                if !parents
                    .get(parent)
                    .is_some_and(|value: &&FileEntry| value.directory && value.proof.is_some())
                {
                    return Err("音声だけの削除の保存された実体・親観測が不正です".into());
                }
            }
            if entry.path == root && !entry.directory {
                return Err("音声観測の会議ルートが不正です".into());
            }
            parents.insert(entry.path.as_str(), entry);
        }
        self.entries()?;
        self.hash()?;
        self.encode()?;
        Ok(())
    }
    fn entries(&self) -> Result<Vec<Entry>, String> {
        let entries = self
            .manifest
            .entries
            .iter()
            .filter(|entry| !entry.directory)
            .map(|entry| {
                Ok(Entry {
                    relative_path: entry.path.clone(),
                    asset_id: entry.registered.as_ref().map(|asset| asset.id.clone()),
                    disposition: disposition(entry, &self.records)?,
                    present: entry.proof.is_some(),
                    bytes: entry.proof.as_ref().map(|proof| proof.size),
                    sha256: entry.sha256.clone(),
                    changed: entry.registered.as_ref().is_some_and(|asset| {
                        entry.proof.as_ref().is_some_and(|proof| {
                            u64::try_from(asset.size).ok() != Some(proof.size)
                                || entry.sha256.as_deref() != Some(asset.sha256.as_str())
                        })
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let represented = entries
            .iter()
            .filter(|entry| entry.disposition == Disposition::RegisteredAudio)
            .map(|entry| entry.relative_path.as_str())
            .collect::<BTreeSet<_>>();
        if represented.len() != self.records.audio_assets.len()
            || self
                .records
                .audio_assets
                .keys()
                .any(|path| !represented.contains(path.as_str()))
        {
            return Err("音声だけの削除対象に登録資産の欠落があります".into());
        }
        Ok(entries)
    }
    fn retained(&self) -> Result<Retained, String> {
        let rows = |name: &str| {
            self.scope
                .tables
                .iter()
                .find(|table| table.name == name)
                .map(|table| table.rows)
                .ok_or_else(|| format!("保持する{name}の検査証拠がありません"))
        };
        Ok(Retained {
            analysis_revisions: rows("analysis_revisions")?,
            utterances: rows("utterances")?,
            minutes_revisions: rows("minutes_revisions")?,
            drafts: rows("working_drafts")?,
            edit_operations: rows("edit_operations")?,
            evidence_links: rows("evidence_links")?,
            jobs: rows("processing_jobs")?,
            external_export_records: rows("export_records")?,
        })
    }
    fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("音声だけの削除の保存観測が容量上限を超えています".into());
        }
        Ok(bytes)
    }
    fn hash(&self) -> Result<String, String> {
        let manifest = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
        // Preserve the original read-only review fingerprint across persistence.
        let encoded = serde_json::to_vec(&(
            1,
            "audio_only",
            format!("{:x}", Sha256::digest(manifest)),
            &self.scope,
            &self.records,
            self.entries()?,
            self.retained()?,
        ))
        .map_err(|e| e.to_string())?;
        if encoded.len() > 64 * 1024 * 1024 {
            return Err("音声だけの削除の検査証拠が容量上限を超えています".into());
        }
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("音声だけの削除の対象検査を取り消しました。ファイルと履歴は保持します".into())
    } else {
        Ok(())
    }
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
fn records(db: &Connection, meeting: &str, check: &impl Fn() -> bool) -> Result<Records, String> {
    let mut statement = db.prepare("SELECT t.id,t.kind,t.state,count(c.id),coalesce(max(c.end_us),0) FROM audio_tracks t LEFT JOIN audio_chunks c ON c.meeting_id=t.meeting_id AND c.track_id=t.id WHERE t.meeting_id=?1 GROUP BY t.id ORDER BY t.id").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![meeting], |r| {
            Ok(Track {
                id: r.get(0)?,
                kind: r.get(1)?,
                state: r.get(2)?,
                chunks: u64::try_from(r.get::<_, i64>(3)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                duration_us: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut tracks = Vec::new();
    for row in rows {
        cancel(check)?;
        let track = row.map_err(|e| e.to_string())?;
        if !uuid(&track.id) || tracks.len() >= MAX_ENTRIES {
            return Err("音声トラックの識別子・件数が不正です".into());
        }
        tracks.push(track);
    }
    // Chunk references and original/converted import copies must be accounted
    // for even when their paths are not the current producer layout. Audio MIME
    // is additional evidence, not permission to follow a path outside Storage.
    let mut statement = db.prepare("SELECT f.id,f.relative_path,f.media_type, EXISTS(SELECT 1 FROM audio_chunks c WHERE c.meeting_id=f.meeting_id AND c.file_asset_id=f.id) OR EXISTS(SELECT 1 FROM audio_imports i WHERE i.meeting_id=f.meeting_id AND (i.original_asset_id=f.id OR i.converted_asset_id=f.id)), EXISTS(SELECT 1 FROM audio_import_commits p WHERE p.meeting_id=f.meeting_id AND p.manifest_asset_id=f.id) OR EXISTS(SELECT 1 FROM recording_sessions s WHERE s.meeting_id=f.meeting_id AND s.audio_manifest_hash=f.content_hash) FROM file_assets f WHERE f.meeting_id=?1 ORDER BY f.relative_path").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![meeting], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut audio_assets = BTreeMap::new();
    let mut count = 0;
    for row in rows {
        cancel(check)?;
        count += 1;
        let (id, path, media, referenced, manifest) = row.map_err(|e| e.to_string())?;
        if count > MAX_ENTRIES || !uuid(&id) {
            return Err("音声資産の識別子・件数が不正です".into());
        }
        if referenced && !is_audio_media(&media) {
            return Err("音声参照と登録資産の種類が一致しません。対象を保持します".into());
        }
        if manifest && (referenced || is_audio_media(&media)) {
            return Err(
                "保持する音声manifestと削除する音声資産が競合しています。対象を保持します".into(),
            );
        }
        if is_audio_media(&media) && audio_assets.insert(path, id).is_some() {
            return Err("音声資産の対象パスが重複しています".into());
        }
    }
    Ok(Records {
        tracks,
        audio_assets,
    })
}

fn disposition(entry: &FileEntry, records: &Records) -> Result<Disposition, String> {
    if let Some(id) = records.audio_assets.get(&entry.path) {
        if !entry
            .registered
            .as_ref()
            .is_some_and(|asset| &asset.id == id)
        {
            return Err("音声資産と対象検査の登録証拠が一致しません".into());
        }
        return Ok(Disposition::RegisteredAudio);
    }
    // An explicitly registered non-audio object stays retained even if its
    // filename resembles PCM. Do not erase results, logs or manifests.
    if entry.registered.is_some() {
        return Ok(Disposition::Retain);
    }
    let leaf = entry
        .path
        .rsplit('/')
        .next()
        .ok_or("音声対象のパスが不正です")?;
    match entry.stage_hint.as_deref() {
        Some("recording") if matches!(leaf, "microphone.caf" | "system.caf") => {
            Ok(Disposition::ProducerAudioLeftover)
        }
        Some("import")
            if matches!(
                leaf,
                "original.wav"
                    | "original.mp3"
                    | "original.m4a"
                    | "original.flac"
                    | "audio.flac"
                    | "decoded.caf"
            ) =>
        {
            Ok(Disposition::ProducerAudioLeftover)
        }
        Some("transcription" | "diarization") if leaf == "input.wav" => {
            Ok(Disposition::AnalysisPcm)
        }
        Some(_) => Ok(Disposition::Retain),
        None => Ok(Disposition::Unclassified),
    }
}
fn audio(disposition: &Disposition) -> bool {
    matches!(
        disposition,
        Disposition::RegisteredAudio
            | Disposition::ProducerAudioLeftover
            | Disposition::AnalysisPcm
    )
}

// The private, server-held observation fixes retained objects as well as audio.
// It is deliberately NOT a Prepared meeting deletion or removal capability.
pub(crate) struct Census {
    meeting: MeetingCensus,
    records: Records,
    entries: Vec<Entry>,
    retained: Retained,
}
impl Census {
    pub(crate) fn capture(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        storage.require_data_writable()?;
        cancel(check)?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let result = Self::capture_locked(storage, &db, meeting, "", check)?;
        storage.require_data_writable()?;
        Ok(result)
    }
    fn capture_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        // Do not rely only on the cached fence flag: a newly persisted intent
        // or a different kind of maintenance also invalidates a fresh review.
        if super::fence::other_pending(
            db,
            (!omitted_operation.is_empty()).then_some(omitted_operation),
        )? {
            return Err("未完了の保存領域操作があります。音声の対象確認を開始できません".into());
        }
        let census = MeetingCensus::capture_locked(storage, db, meeting, omitted_operation, check)?;
        let records = records(db, meeting, check)?;
        let mut entries = Vec::new();
        for entry in &census.manifest().entries {
            cancel(check)?;
            if entry.directory {
                continue;
            }
            let disposition = disposition(entry, &records)?;
            let changed = entry.registered.as_ref().is_some_and(|asset| {
                entry.proof.as_ref().is_some_and(|proof| {
                    u64::try_from(asset.size).ok() != Some(proof.size)
                        || entry.sha256.as_deref() != Some(asset.sha256.as_str())
                })
            });
            entries.push(Entry {
                relative_path: entry.path.clone(),
                asset_id: entry.registered.as_ref().map(|asset| asset.id.clone()),
                disposition,
                present: entry.proof.is_some(),
                bytes: entry.proof.as_ref().map(|proof| proof.size),
                sha256: entry.sha256.clone(),
                changed,
            });
        }
        // Every declared audio asset, including missing files, appears exactly
        // once. No neighboring meeting, external export or original is opened.
        let represented = entries
            .iter()
            .filter(|entry| entry.disposition == Disposition::RegisteredAudio)
            .map(|entry| entry.relative_path.as_str())
            .collect::<BTreeSet<_>>();
        if represented.len() != records.audio_assets.len()
            || records
                .audio_assets
                .keys()
                .any(|path| !represented.contains(path.as_str()))
        {
            return Err("音声だけの削除対象に登録資産の欠落があります".into());
        }
        let rows = |name: &str| {
            census
                .scope()
                .tables
                .iter()
                .find(|table| table.name == name)
                .map(|table| table.rows)
                .ok_or_else(|| format!("保持する{name}の検査証拠がありません"))
        };
        let retained = Retained {
            analysis_revisions: rows("analysis_revisions")?,
            utterances: rows("utterances")?,
            minutes_revisions: rows("minutes_revisions")?,
            drafts: rows("working_drafts")?,
            edit_operations: rows("edit_operations")?,
            evidence_links: rows("evidence_links")?,
            jobs: rows("processing_jobs")?,
            external_export_records: rows("export_records")?,
        };
        let current =
            MeetingCensus::capture_locked(storage, db, meeting, omitted_operation, check)?;
        if current.hash()? != census.hash()?
            || current.scope() != census.scope()
            || self::records(db, meeting, check)? != records
        {
            return Err("音声だけの削除の検査中に対象・保持履歴が変更されました".into());
        }
        cancel(check)?;
        Ok(Self {
            meeting: census,
            records,
            entries,
            retained,
        })
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            scope: self.meeting.scope().clone(),
            manifest: self.meeting.manifest().clone(),
            records: self.records.clone(),
        }
    }
    fn hash(&self) -> Result<String, String> {
        // Typed discriminator prevents reuse as a whole-meeting/model review.
        let encoded = serde_json::to_vec(&(
            1,
            "audio_only",
            self.meeting.hash()?,
            self.meeting.scope(),
            &self.records,
            &self.entries,
            &self.retained,
        ))
        .map_err(|e| e.to_string())?;
        if encoded.len() > 64 * 1024 * 1024 {
            return Err("音声だけの削除の検査証拠が容量上限を超えています".into());
        }
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
    pub(crate) fn verify(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let current = Self::capture(storage, &self.meeting.manifest().meeting_id, check)?;
        if current.hash()? != self.hash()? {
            return Err("確認後に音声対象・保持履歴が変更されています。再確認してください".into());
        }
        Ok(())
    }
    pub(crate) fn preview(&self, inspection: &str) -> Result<Preview, String> {
        if !uuid(inspection) {
            return Err("音声だけの削除の検査IDが不正です".into());
        }
        let mut bytes = 0_u64;
        let targets = self
            .entries
            .iter()
            .filter(|entry| audio(&entry.disposition))
            .collect::<Vec<_>>();
        for entry in &targets {
            bytes = bytes
                .checked_add(entry.bytes.unwrap_or(0))
                .ok_or("音声容量が範囲外です")?;
        }
        Ok(Preview {
            schema_version: 1,
            inspection_id: inspection.into(),
            meeting_id: self.meeting.manifest().meeting_id.clone(),
            census_hash: self.hash()?,
            tracks: self.records.tracks.clone(),
            retained: self.retained.clone(),
            entries: self.entries.clone(),
            audio_files: targets.len(),
            audio_bytes: bytes,
            missing_audio_files: targets.iter().filter(|entry| !entry.present).count(),
            changed_audio_files: targets.iter().filter(|entry| entry.changed).count(),
            unregistered_audio_files: targets
                .iter()
                .filter(|entry| entry.asset_id.is_none())
                .count(),
            unclassified_files: self
                .entries
                .iter()
                .filter(|entry| entry.disposition == Disposition::Unclassified)
                .count(),
            execute_allowed: false,
        })
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
