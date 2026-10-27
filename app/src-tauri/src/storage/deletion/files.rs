//! Read-only deletion census. Serialized manifests are observations, never
//! authority to unlink. Consent, persistent fencing and removal follow.
use super::{
    database::Scope,
    fs::{Directory, Identity, Proof},
};
use crate::storage::Storage;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

pub(super) const MAX_ENTRIES: usize = 200_000;
pub(super) const MAX_RESERVATION_ROOTS: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_MANIFEST: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Registered {
    pub id: String,
    pub size: i64,
    pub sha256: String,
    pub state: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Entry {
    pub path: String,
    pub directory: bool,
    pub proof: Option<Proof>,
    pub sha256: Option<String>,
    pub registered: Option<Registered>,
    // A known producer layout explains a leftover; it does not prove ownership.
    pub stage_hint: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Manifest {
    pub version: u32,
    pub database_scope_hash: String,
    pub meeting_id: String,
    pub root: Identity,
    pub database: Identity,
    pub entries: Vec<Entry>,
    // Exact siblings derived from the preceding canonical removal plan only.
    // Their presence in serialized observations is not deletion authority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reservation_roots: Vec<String>,
}
impl Manifest {
    pub(super) fn directory_scope(&self) -> bool {
        self.version == 3
    }
    pub(super) fn validate_roots(&self) -> Result<(), String> {
        if !matches!(
            (self.version, self.reservation_roots.is_empty()),
            (1, true) | (2, false) | (3, true)
        ) || self.reservation_roots.len() > MAX_RESERVATION_ROOTS
            || self
                .reservation_roots
                .iter()
                .any(|p| !valid_reservation_root(p))
            || self.reservation_roots.windows(2).any(|w| w[0] >= w[1])
        {
            return Err("削除予約ルートの形式・順序・件数が不正です".into());
        }
        Ok(())
    }
    pub(super) fn is_tree_root(&self, path: &str) -> bool {
        path == format!("meetings/{}", self.meeting_id)
            || self.reservation_roots.iter().any(|r| r == path)
    }
    pub(super) fn valid_observed_path(&self, path: &str) -> bool {
        path.len() <= 256
            && !path.contains(['\\', '\0'])
            && path
                .split('/')
                .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
            && (self.is_tree_root(path)
                || valid_path(path, &self.meeting_id)
                || self
                    .reservation_roots
                    .iter()
                    .any(|r| path.starts_with(&format!("{r}/"))))
    }
}
pub(super) fn valid_reservation_root(path: &str) -> bool {
    let Some(name) = path.strip_prefix("meetings/.mon-delete-") else {
        return false;
    };
    let Some((prefix, index)) = name.rsplit_once('-') else {
        return false;
    };
    if !index
        .parse::<usize>()
        .is_ok_and(|n| n < MAX_ENTRIES && n.to_string() == index)
    {
        return false;
    }
    if uuid(prefix) {
        return true;
    }
    let Some((operation, generation)) = prefix.rsplit_once("-r") else {
        return false;
    };
    uuid(operation)
        && generation
            .parse::<u32>()
            .is_ok_and(|n| (1..=32).contains(&n) && n.to_string() == generation)
}
#[derive(Debug)]
pub(super) struct Census {
    manifest: Manifest,
    scope: Scope,
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除対象ファイルの検査を取り消しました".into())
    } else {
        Ok(())
    }
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
// Exact native producer grammar, not an extension-based audio guess. The
// containing directory must still belong to a DB recording session. Active
// (not yet sealed) segments use the same CAF name and may contain partial PCM.
pub(super) fn raw_recording_audio_name(leaf: &str) -> bool {
    raw_recording_sequence(leaf, ".caf").is_some()
}
fn raw_recording_sequence(leaf: &str, suffix: &str) -> Option<u32> {
    let sequence = leaf
        .strip_prefix("raw-microphone-")
        .or_else(|| leaf.strip_prefix("raw-system-"))?
        .strip_suffix(suffix)?;
    if sequence.len() != 8 || !sequence.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    sequence.parse().ok().filter(|n| (1..=32_768).contains(n))
}
fn raw_recording_metadata_name(leaf: &str) -> bool {
    matches!(
        leaf,
        "raw-microphone.json"
            | "raw-system.json"
            | "final-flac.json"
            | "partial-flac.json"
            | "partial-recovery.json"
            | ".final-flac.lock"
    ) || raw_recording_sequence(leaf, ".json").is_some()
}
pub(super) fn valid_path(s: &str, meeting: &str) -> bool {
    s.len() <= 256
        && s.starts_with(&format!("meetings/{meeting}/"))
        && !s.contains(['\\', '\0'])
        && s.split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
}
// Syntax is not authority. Exact membership must come from a typed source or
// removal receipt before these paths are used for a filesystem observation.
pub(super) fn valid_audio_reservation_path(path: &str, meeting: &str) -> bool {
    let Some((parent, name)) = path.rsplit_once('/') else {
        return false;
    };
    let Some(name) = name.strip_prefix(".mon-delete-audio-") else {
        return false;
    };
    path.len() <= 512
        && valid_path(parent, meeting)
        && valid_reservation_root(&format!("meetings/.mon-delete-{name}"))
}
pub(super) struct LongAudioPaths(BTreeSet<String>);
impl LongAudioPaths {
    pub(super) fn from_history(paths: &BTreeSet<String>, meeting: &str) -> Result<Self, String> {
        if paths.len() > MAX_ENTRIES {
            return Err("音声の履歴パス数が安全上限を超えています".into());
        }
        let mut long = BTreeSet::new();
        for path in paths.iter().filter(|p| p.len() > 256) {
            if !valid_audio_reservation_path(path, meeting) {
                return Err("長い音声予約パスの形式が不正です".into());
            }
            long.insert(path.clone());
        }
        Ok(Self(long))
    }
    pub(super) fn contains(&self, path: &str) -> bool {
        self.0.contains(path)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
fn hash(s: &[u8]) -> String {
    format!("{:x}", Sha256::digest(s))
}

#[derive(Default)]
struct Records {
    assets: BTreeMap<String, Registered>,
    sessions: BTreeSet<String>,
    imports: BTreeSet<String>,
    jobs: BTreeMap<String, String>,
    flac_files: crate::storage::recordings::flac::WorkspaceFiles,
}
impl Records {
    fn read(db: &Connection, meeting: &str) -> Result<Self, String> {
        let mut q=db.prepare("SELECT id,relative_path,size_bytes,content_hash,state FROM file_assets WHERE meeting_id=?1 ORDER BY relative_path").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![meeting], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    Registered {
                        id: r.get(0)?,
                        size: r.get(2)?,
                        sha256: r.get(3)?,
                        state: r.get(4)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut assets = BTreeMap::new();
        for row in rows {
            let (path, asset) = row.map_err(|e| e.to_string())?;
            if assets.len() >= MAX_ENTRIES
                || !valid_path(&path, meeting)
                || asset.size < 0
                || asset.sha256.len() != 64
                || !asset
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("登録資産の管理パス・SHA・件数が不正です".into());
            }
            if assets.insert(path, asset).is_some() {
                return Err("登録資産のパスが重複しています".into());
            }
        }
        let ids = |table: &str| -> Result<BTreeSet<String>, String> {
            db.prepare(&format!(
                "SELECT id FROM {table} WHERE meeting_id=?1 ORDER BY id"
            ))
            .map_err(|e| e.to_string())?
            .query_map(params![meeting], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
        };
        let jobs = db
            .prepare("SELECT id,kind FROM processing_jobs WHERE meeting_id=?1 ORDER BY id")
            .map_err(|e| e.to_string())?
            .query_map(params![meeting], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            assets,
            sessions: ids("recording_sessions")?,
            imports: ids("audio_imports")?,
            jobs,
            flac_files: Default::default(),
        })
    }
    fn hint(&self, path: &str) -> Option<String> {
        let p: Vec<_> = path.split('/').collect();
        if p.len() == 5 && uuid(p[3]) {
            if p[2] == "audio"
                && self.sessions.contains(p[3])
                && (matches!(
                    p[4],
                    "microphone.caf"
                        | "system.caf"
                        | "manifest.json"
                        | "input-events.json"
                        | "input-notices.json"
                        | "input-notices.lock"
                ) || raw_recording_audio_name(p[4])
                    || raw_recording_metadata_name(p[4]))
            {
                return Some("recording".into());
            }
            if p[2] == "imports"
                && self.imports.contains(p[3])
                && matches!(
                    p[4],
                    "original.wav"
                        | "original.mp3"
                        | "original.m4a"
                        | "original.flac"
                        | "audio.flac"
                        | "decoded.caf"
                        | "manifest.json"
                )
            {
                return Some("import".into());
            }
        }
        if p.len() == 6 && p[2] == "analysis" && uuid(p[3]) && unit(p[4]) {
            let kind = self.jobs.get(p[3])?;
            let allowed = match kind.as_str() {
                "transcription" => matches!(
                    p[5],
                    "input.wav"
                        | "result.json"
                        | "logical-result.json"
                        | "stdout.log"
                        | "stderr.log"
                ),
                "diarization" => matches!(
                    p[5],
                    "input.wav"
                        | "result.json"
                        | "stdout.log"
                        | "logical-stdout.log"
                        | "stderr.log"
                ),
                "minutes_generation" => matches!(
                    p[5],
                    "system.txt"
                        | "prompt.txt"
                        | "schema.json"
                        | "result.json"
                        | "items.json"
                        | "stderr.log"
                ),
                _ => false,
            };
            if allowed {
                return Some(kind.clone());
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Summary {
    pub files: usize,
    pub directories: usize,
    pub missing: usize,
    pub changed: usize,
    pub unregistered_with_hint: usize,
    pub unknown: usize,
    pub bytes: u64,
}
fn unit(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("unit-") else {
        return false;
    };
    let Some((index, id)) = rest.split_once('-') else {
        return false;
    };
    index.parse::<usize>().is_ok_and(|n| n.to_string() == index) && uuid(id)
}

fn walk(
    dir: &Directory,
    path: &str,
    depth: usize,
    entries: &mut BTreeMap<String, Entry>,
    records: &Records,
    check: &impl Fn() -> bool,
    reserved: Option<&str>,
    long_audio: Option<&LongAudioPaths>,
) -> Result<(), String> {
    cancel(check)?;
    if depth > MAX_DEPTH
        || entries.len() >= MAX_ENTRIES
        || (path.len() > 256 && Some(path) != reserved)
    {
        return Err("削除対象の深さ・パス・件数が安全上限を超えています".into());
    }
    let before = dir.proof()?;
    entries.insert(
        path.into(),
        Entry {
            path: path.into(),
            directory: true,
            proof: Some(before.clone()),
            sha256: None,
            registered: None,
            stage_hint: None,
        },
    );
    let names = dir.names(MAX_ENTRIES - entries.len())?;
    for name in &names {
        cancel(check)?;
        let child_path = format!("{path}/{name}");
        let long_allowed = long_audio.is_some_and(|p| p.contains(&child_path));
        if (child_path.len() > 256 && Some(child_path.as_str()) != reserved && !long_allowed)
            || entries.len() >= MAX_ENTRIES
        {
            return Err("削除対象のパス・件数が安全上限を超えています".into());
        }
        if dir.kind(name)? {
            if long_allowed {
                return Err("長い音声予約パスへフォルダーを採用しません".into());
            }
            let child = dir
                .child(name)?
                .ok_or("検査中に対象フォルダーが消失しました")?;
            walk(
                &child,
                &child_path,
                depth + 1,
                entries,
                records,
                check,
                reserved,
                long_audio,
            )?;
        } else {
            let (before, sha256) = super::verification::hash(dir, name, check)?;
            let file = dir.regular(name)?;
            entries.insert(
                child_path.clone(),
                Entry {
                    path: child_path.clone(),
                    directory: false,
                    proof: Some(before),
                    sha256: Some(sha256),
                    registered: records.assets.get(&child_path).cloned(),
                    stage_hint: {
                        let hint = records.flac_files.hint(&child_path, &file)?;
                        let hint = hint.or_else(|| records.hint(&child_path));
                        hint
                    },
                },
            );
        }
    }
    if dir.proof()? != before || dir.names(MAX_ENTRIES)? != names {
        return Err("検査中にフォルダーの内容が変更されました".into());
    }
    Ok(())
}
fn inventory(
    root: &Directory,
    meeting: &str,
    records: &Records,
    check: &impl Fn() -> bool,
    reserved: Option<&str>,
    reservation_roots: &[String],
    long_audio: Option<&LongAudioPaths>,
) -> Result<Vec<Entry>, String> {
    if reservation_roots.len() > MAX_RESERVATION_ROOTS
        || reservation_roots.iter().any(|p| !valid_reservation_root(p))
        || reservation_roots.windows(2).any(|w| w[0] >= w[1])
    {
        return Err("予約ルートの読取範囲が不正です".into());
    }
    root.verify()?;
    let mut entries = BTreeMap::new();
    if let Some(meetings) = root.child("meetings")? {
        if let Some(directory) = meetings.child(meeting)? {
            walk(
                &directory,
                &format!("meetings/{meeting}"),
                0,
                &mut entries,
                records,
                check,
                reserved,
                long_audio,
            )?;
        }
        // Never enumerate siblings. Only canonical, exact plan-derived names
        // are opened relative to the pinned meetings descriptor, no links.
        for path in reservation_roots {
            cancel(check)?;
            let leaf = path
                .strip_prefix("meetings/")
                .ok_or("予約ルートが不正です")?;
            if let Some(directory) = meetings.child(leaf)? {
                walk(
                    &directory,
                    path,
                    0,
                    &mut entries,
                    &Records::default(),
                    check,
                    reserved,
                    long_audio,
                )?;
            }
        }
    }
    for (path, asset) in &records.assets {
        if !entries.contains_key(path) {
            if entries.len() >= MAX_ENTRIES {
                return Err("削除対象の件数が安全上限を超えています".into());
            }
            entries.insert(
                path.clone(),
                Entry {
                    path: path.clone(),
                    directory: false,
                    proof: None,
                    sha256: None,
                    registered: Some(asset.clone()),
                    stage_hint: None,
                },
            );
        } else if entries[path].directory {
            return Err("登録ファイルのパスがフォルダーへ変更されています".into());
        }
    }
    root.verify()?;
    Ok(entries.into_values().collect())
}

impl Census {
    pub(super) fn capture_directory(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let db = crate::operation_observation::lock(&storage.connection)?;
        Self::capture_directory_locked(storage, &db, meeting, "", check)
    }
    pub(super) fn capture_directory_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        storage.managed_anchor.verify_current_history()?;
        let scope = Scope::capture(db, meeting, operation, check)?;
        let (root, database) = storage.managed_anchor.identity()?;
        let proof = storage
            .managed_anchor
            .directory_ref()?
            .child("meetings")?
            .map(|parent| parent.child(meeting))
            .transpose()?
            .flatten()
            .map(|dir| dir.proof().map(|proof| proof.directory_scope()))
            .transpose()?;
        let entries = proof
            .into_iter()
            .map(|proof| Entry {
                path: format!("meetings/{meeting}"),
                directory: true,
                proof: Some(proof),
                sha256: None,
                registered: None,
                stage_hint: None,
            })
            .collect();
        let result = Self {
            manifest: Manifest {
                version: 3,
                database_scope_hash: scope.hash()?,
                meeting_id: meeting.into(),
                root,
                database,
                entries,
                reservation_roots: vec![],
            },
            scope,
        };
        result.encode()?;
        Ok(result)
    }

    // A deleted meeting cannot be looked up as a fresh deletion scope. Require
    // its canonical compound receipt, prove all original rows absent, and keep
    // that original scope hash. Reappeared files have no resurrected ownership.
    pub(super) fn capture_deleted_locked(
        storage: &Storage,
        db: &Connection,
        prepared: &super::intent::Prepared,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        reservation_roots: &[String],
    ) -> Result<Self, String> {
        cancel(check)?;
        super::intent::read_database(db, prepared.operation(), check)?.matches_saved(prepared)?;
        if reserved.is_some_and(|s| s.len() > 512 || s.contains(['\\', '\0']))
            || storage
                .maintenance
                .load(std::sync::atomic::Ordering::Acquire)
            || storage.retired.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("削除済み範囲を確認する保存領域の状態が一致しません".into());
        }
        let scope = prepared.scope();
        super::startup::verify_completed_rows(storage, db, prepared, check)?;
        let (root, database) = storage.managed_anchor.identity()?;
        if (root.clone(), database.clone())
            != (
                prepared.manifest().root.clone(),
                prepared.manifest().database.clone(),
            )
        {
            let (active, finished) = super::startup::review_manifest(db, prepared, check)?;
            super::renewal::verify_review_identity(storage, db, prepared, &active, finished)?;
        }
        let records = Records::read(db, &scope.meeting_id)?;
        let directory = storage.managed_anchor.directory_ref()?;
        let entries = inventory(
            directory,
            &scope.meeting_id,
            &records,
            check,
            reserved,
            reservation_roots,
            None,
        )?;
        if inventory(
            directory,
            &scope.meeting_id,
            &records,
            check,
            reserved,
            reservation_roots,
            None,
        )? != entries
        {
            return Err("削除済み範囲の再確認中に対象が変更されました".into());
        }
        super::startup::verify_completed_rows(storage, db, prepared, check)?;
        super::intent::read_database(db, prepared.operation(), check)?.matches_saved(prepared)?;
        if storage.managed_anchor.identity()? != (root.clone(), database.clone()) {
            return Err("再確認中に管理領域・DBが変更されました".into());
        }
        let result = Self {
            scope: scope.clone(),
            manifest: Manifest {
                version: if reservation_roots.is_empty() { 1 } else { 2 },
                database_scope_hash: scope.hash()?,
                meeting_id: scope.meeting_id.clone(),
                root,
                database,
                entries,
                reservation_roots: reservation_roots.to_vec(),
            },
        };
        result.encode()?;
        Ok(result)
    }
    pub(super) fn capture(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_removal_unlocked(storage, meeting, "", check, None, &[])
    }
    // The durable-intent transaction already holds the connection mutex.
    pub(super) fn capture_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_removal_locked(storage, db, meeting, omitted_operation, check, None, &[])
    }
    // The exact reserved sibling from an armed receipt is a read allowance,
    // not authority to adopt or delete an unregistered object.
    pub(super) fn capture_removal_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        reservation_roots: &[String],
    ) -> Result<Self, String> {
        storage.managed_anchor.verify_current_history()?;
        Self::capture_scoped_locked(
            storage,
            db,
            meeting,
            check,
            reserved,
            reservation_roots,
            None,
            false,
            &|| Scope::capture(db, meeting, omitted_operation, check),
        )
    }
    // Whole-meeting removal deletes sidecars too. Its classification must not
    // depend on checkpoints that legitimately disappear during that removal.
    pub(super) fn capture_audio_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_scoped_locked(storage, db, meeting, check, None, &[], None, true, &|| {
            Scope::capture(db, meeting, omitted_operation, check)
        })
    }
    pub(super) fn capture_audio_reconfirmation_locked(
        storage: &Storage,
        db: &Connection,
        source: &super::audio::intent::reconfirmation::Source,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
    ) -> Result<Self, String> {
        source.verify_review_anchor(storage, db, check)?;
        let long = LongAudioPaths::from_history(&source.paths(), source.meeting())?;
        Self::capture_scoped_locked(
            storage,
            db,
            source.meeting(),
            check,
            reserved,
            &[],
            Some(&long),
            true,
            &|| {
                Scope::capture_audio_reconfirmation(
                    db,
                    source.meeting(),
                    omitted_operation,
                    source,
                    check,
                )
            },
        )
    }
    pub(super) fn capture_audio_remaining_locked(
        storage: &Storage,
        db: &Connection,
        receipt: &super::audio::intent::removal::Receipt,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
    ) -> Result<Self, String> {
        receipt.verify_saved(db, check)?;
        let saved = receipt.source_saved();
        let source = receipt.reconfirmation_source(db, check)?;
        let mut paths = receipt.source_paths();
        if let Some(source) = &source {
            source.verify_review_anchor(storage, db, check)?;
            paths.extend(source.paths());
        }
        let long = LongAudioPaths::from_history(&paths, saved.meeting())?;
        Self::capture_scoped_locked(
            storage,
            db,
            saved.meeting(),
            check,
            reserved,
            &[],
            Some(&long),
            true,
            &|| match &source {
                Some(source) => Scope::capture_audio_reconfirmation(
                    db,
                    saved.meeting(),
                    saved.operation(),
                    source,
                    check,
                ),
                None => Scope::capture(db, saved.meeting(), saved.operation(), check),
            },
        )
    }
    // Load DB evidence on either side of the I/O; no shared connection is
    // borrowed while reading audio. The durable deletion fence remains armed.
    #[allow(clippy::too_many_arguments)]
    fn capture_scoped_unlocked(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        roots: &[String],
        long: Option<&LongAudioPaths>,
        inspect_flac: bool,
        allow_restart: bool,
        capture_scope: &impl Fn(&Connection) -> Result<Scope, String>,
    ) -> Result<Self, String> {
        cancel(check)?;
        if reserved.is_some_and(|s| s.len() > 512 || s.contains(['\\', '\0']))
            || (!allow_restart && storage.restart_required())
            || storage
                .maintenance
                .load(std::sync::atomic::Ordering::Acquire)
            || storage.retired.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("削除対象の保存領域・予約状態が一致しません".into());
        }
        let metadata = || -> Result<_, String> {
            let db = crate::operation_observation::lock(&storage.connection)?;
            Ok((capture_scope(&db)?, Records::read(&db, meeting)?))
        };
        let (scope, mut records) = metadata()?;
        let (root, database) = storage.managed_anchor.identity()?;
        if inspect_flac {
            records.flac_files = crate::storage::recordings::flac::inspect_workspaces(
                storage,
                meeting,
                &records.sessions,
                check,
            )?;
        }
        let directory = storage.managed_anchor.directory_ref()?;
        let entries = inventory(directory, meeting, &records, check, reserved, roots, long)?;
        if inventory(directory, meeting, &records, check, reserved, roots, long)? != entries {
            return Err("検査中に削除対象が変更されました".into());
        }
        if inspect_flac
            && crate::storage::recordings::flac::inspect_workspaces(
                storage,
                meeting,
                &records.sessions,
                check,
            )? != records.flac_files
        {
            return Err("検査中にFLACの所有状態が変更されました".into());
        }
        let (after, rows) = metadata()?;
        if after != scope
            || rows.assets != records.assets
            || rows.sessions != records.sessions
            || rows.imports != records.imports
            || rows.jobs != records.jobs
            || storage.managed_anchor.identity()? != (root.clone(), database.clone())
        {
            return Err("検査中に削除のDB範囲・保存領域が変更されました".into());
        }
        let result = Self {
            manifest: Manifest {
                version: if roots.is_empty() { 1 } else { 2 },
                database_scope_hash: scope.hash()?,
                meeting_id: meeting.into(),
                root,
                database,
                entries,
                reservation_roots: roots.to_vec(),
            },
            scope,
        };
        result.encode()?;
        Ok(result)
    }
    pub(super) fn capture_audio_unlocked(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::capture_scoped_unlocked(
            storage,
            meeting,
            check,
            None,
            &[],
            None,
            true,
            false,
            &|db| Scope::capture(db, meeting, "", check),
        )
    }
    pub(super) fn capture_removal_unlocked(
        storage: &Storage,
        meeting: &str,
        operation: &str,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        roots: &[String],
    ) -> Result<Self, String> {
        storage.managed_anchor.verify_current_history()?;
        Self::capture_scoped_unlocked(
            storage,
            meeting,
            check,
            reserved,
            roots,
            None,
            false,
            false,
            &|db| Scope::capture(db, meeting, operation, check),
        )
    }
    pub(super) fn capture_deleted_unlocked(
        storage: &Storage,
        prepared: &super::intent::Prepared,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        roots: &[String],
    ) -> Result<Self, String> {
        Self::capture_scoped_unlocked(
            storage,
            &prepared.scope().meeting_id,
            check,
            reserved,
            roots,
            None,
            false,
            true,
            &|db| {
                super::intent::read_database(db, prepared.operation(), check)?
                    .matches_saved(prepared)?;
                super::startup::verify_completed_rows(storage, db, prepared, check)?;
                let actual = storage.managed_anchor.identity()?;
                if actual
                    != (
                        prepared.manifest().root.clone(),
                        prepared.manifest().database.clone(),
                    )
                {
                    let (active, finished) = super::startup::review_manifest(db, prepared, check)?;
                    super::renewal::verify_review_identity(
                        storage, db, prepared, &active, finished,
                    )?;
                }
                Ok(prepared.scope().clone())
            },
        )
    }
    pub(super) fn capture_audio_remaining_unlocked(
        storage: &Storage,
        receipt: &super::audio::intent::removal::Receipt,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
    ) -> Result<Self, String> {
        let source = {
            let db = crate::operation_observation::lock(&storage.connection)?;
            receipt.reconfirmation_source(&db, check)?
        };
        let saved = receipt.source_saved();
        let mut paths = receipt.source_paths();
        if let Some(source) = &source {
            paths.extend(source.paths());
        }
        let long = LongAudioPaths::from_history(&paths, saved.meeting())?;
        Self::capture_scoped_unlocked(
            storage,
            saved.meeting(),
            check,
            reserved,
            &[],
            Some(&long),
            true,
            false,
            &|db| {
                receipt.verify_saved(db, check)?;
                match &source {
                    Some(source) => {
                        source.verify_review_anchor(storage, db, check)?;
                        Scope::capture_audio_reconfirmation(
                            db,
                            saved.meeting(),
                            saved.operation(),
                            source,
                            check,
                        )
                    }
                    None => Scope::capture(db, saved.meeting(), saved.operation(), check),
                }
            },
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn capture_scoped_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        check: &impl Fn() -> bool,
        reserved: Option<&str>,
        reservation_roots: &[String],
        long_audio: Option<&LongAudioPaths>,
        inspect_flac: bool,
        capture_scope: &impl Fn() -> Result<Scope, String>,
    ) -> Result<Self, String> {
        if reserved.is_some_and(|s| s.len() > 512 || s.contains(['\\', '\0'])) {
            return Err("予約パスが安全上限を超えています".into());
        }
        if storage.restart_required() {
            return Err("旧保存先は停止済みです。再起動してください".into());
        }
        if storage
            .maintenance
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("保存領域の操作または書込停止が既に進行中です".into());
        }
        let (root, database) = storage.managed_anchor.identity()?;
        let scope = capture_scope()?;
        let mut records = Records::read(db, meeting)?;
        if inspect_flac {
            records.flac_files = crate::storage::recordings::flac::inspect_workspaces(
                storage,
                meeting,
                &records.sessions,
                check,
            )?;
        }
        // Enumerate through the startup descriptor, never reopen a path and
        // temporarily treat its replacement as the original root.
        let root_directory = storage.managed_anchor.directory_ref()?;
        let entries = inventory(
            root_directory,
            meeting,
            &records,
            check,
            reserved,
            reservation_roots,
            long_audio,
        )?;
        let changed_workspaces = inspect_flac
            && crate::storage::recordings::flac::inspect_workspaces(
                storage,
                meeting,
                &records.sessions,
                check,
            )? != records.flac_files;
        if inventory(
            root_directory,
            meeting,
            &records,
            check,
            reserved,
            reservation_roots,
            long_audio,
        )? != entries
            || changed_workspaces
            || capture_scope()? != scope
            || storage.managed_anchor.identity()? != (root.clone(), database.clone())
        {
            return Err("確認中に削除対象が変更されました".into());
        }
        let result = Self {
            manifest: Manifest {
                version: if reservation_roots.is_empty() { 1 } else { 2 },
                database_scope_hash: scope.hash()?,
                meeting_id: meeting.into(),
                root,
                database,
                entries,
                reservation_roots: reservation_roots.to_vec(),
            },
            scope,
        };
        result.encode()?;
        Ok(result)
    }
    pub(super) fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub(super) fn scope(&self) -> &Scope {
        &self.scope
    }
    pub(super) fn encode(&self) -> Result<Vec<u8>, String> {
        self.manifest.validate_roots()?;
        let bytes = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            Err("ファイルmanifestが安全上限を超えています".into())
        } else {
            Ok(bytes)
        }
    }
    pub(super) fn hash(&self) -> Result<String, String> {
        Ok(hash(&self.encode()?))
    }
    pub(super) fn verify_tree_roots(&self, storage: &Storage) -> Result<(), String> {
        let meetings = storage.managed_anchor.directory_ref()?.child("meetings")?;
        let mut paths = self.manifest.reservation_roots.clone();
        paths.push(format!("meetings/{}", self.manifest.meeting_id));
        for path in paths {
            let actual = meetings
                .as_ref()
                .map(|dir| dir.child(path.strip_prefix("meetings/").ok_or("ルートが不正です")?))
                .transpose()?
                .flatten()
                .map(|dir| dir.proof())
                .transpose()?;
            let expected = self
                .manifest
                .entries
                .iter()
                .find(|e| e.path == path)
                .and_then(|e| e.proof.as_ref());
            if actual.as_ref() != expected {
                return Err(
                    "確認終盤に会議・予約ルートが変更されています。対象を保持します".into(),
                );
            }
        }
        storage.managed_anchor.verify()
    }
    pub(super) fn summary(&self) -> Result<Summary, String> {
        let mut result = Summary {
            files: 0,
            directories: 0,
            missing: 0,
            changed: 0,
            unregistered_with_hint: 0,
            unknown: 0,
            bytes: 0,
        };
        for entry in &self.manifest.entries {
            if entry.directory {
                result.directories += 1;
                continue;
            }
            result.files += 1;
            if let Some(proof) = &entry.proof {
                result.bytes = result
                    .bytes
                    .checked_add(proof.size)
                    .ok_or("削除対象容量が範囲外です")?;
                if let Some(registered) = &entry.registered {
                    if u64::try_from(registered.size).ok() != Some(proof.size)
                        || entry.sha256.as_deref() != Some(registered.sha256.as_str())
                    {
                        result.changed += 1;
                    }
                } else if entry.stage_hint.is_some() {
                    result.unregistered_with_hint += 1;
                } else {
                    result.unknown += 1;
                }
            } else {
                result.missing += 1;
            }
        }
        Ok(result)
    }
    pub(super) fn verify(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let capture = if self.manifest.directory_scope() {
            Self::capture_directory
        } else {
            Self::capture
        };
        let current = capture(storage, &self.scope.meeting_id, check)?;
        if current.scope != self.scope || current.manifest != self.manifest {
            return Err("確認後にDB・ファイル・実体が変更されています。再確認してください".into());
        }
        Ok(())
    }
}
