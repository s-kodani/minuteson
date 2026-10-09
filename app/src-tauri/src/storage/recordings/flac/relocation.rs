//! Physical correspondence for producer workspaces, not portable ownership.
//! Source and Destination can only be minted from a held native snapshot and
//! its owned placement. Persisted facts are admitted behind bootstrap's live
//! placement lease, and cannot mint either capability or delete consent.
use super::*;
use crate::bootstrap::placement::{NativeHistoryAdmission, Placement};
use crate::storage::backup::relocation::NativeSnapshot;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod recovery;
pub(super) use recovery::{recovery_input, RecoveryRebind};

const MAX_WORKSPACES: usize = 4096;
const MAX_CLAIMS: usize = 32768;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    id: String,
    checkpoint_hash: String,
    staging_name: String,
    path: String,
    recorded: Identity,
    observed: Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackFacts {
    id: String,
    recorded: Identity,
    observed: Identity,
    bodies: Vec<Body>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    base: String,
    name: String,
    plan_hash: String,
    recorded: Identity,
    observed: Identity,
    tracks: Vec<TrackFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<recovery::Facts>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFacts {
    version: u32,
    source_root: PathBuf,
    root: Identity,
    database: Identity,
    native_hash: String,
    workspaces: Vec<Workspace>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(in crate::storage) struct Source(SourceFacts);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundTrack {
    id: String,
    directory: Identity,
    bodies: Vec<Identity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundWorkspace {
    base: String,
    name: String,
    recording: Identity,
    tracks: Vec<BoundTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_document: Option<Identity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationFacts {
    version: u32,
    operation: String,
    native_hash: String,
    target_root: PathBuf,
    root: Identity,
    database: Identity,
    workspaces: Vec<BoundWorkspace>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(in crate::storage) struct Destination(DestinationFacts);

// No Deserialize implementation: callers receive this only after validating
// a selected, durably admitted native placement and its independent inventory.
pub(in crate::storage) struct Aliases {
    root: PathBuf,
    root_identity: Identity,
    database: Identity,
    plans: BTreeMap<(String, String), (Workspace, BoundWorkspace)>,
    checkpoints: BTreeMap<String, (String, Identity, Identity)>,
}
fn metadata(root: &Path, path: &str) -> Result<Identity, String> {
    Identity::read(&std::fs::symlink_metadata(root.join(path)).map_err(|e| e.to_string())?)
}
fn track_base(base: &str, id: &str) -> Result<String, String> {
    let parts: Vec<_> = base.split('/').collect();
    if parts.len() != 4
        || parts[0] != "meetings"
        || parts[2] != "audio"
        || !uuid(parts[1])
        || !uuid(parts[3])
        || !uuid(id)
    {
        return Err("FLAC移動履歴の所属・管理パスが不正です".into());
    }
    Ok(format!("meetings/{}/audio/{id}", parts[1]))
}
fn valid_source(source: &SourceFacts) -> Result<(), String> {
    source.root.validate(true)?;
    source.database.validate(false)?;
    if source.version != 1
        || !source.source_root.is_absolute()
        || !inventory::valid_hash(&source.native_hash)
        || source.workspaces.is_empty()
        || source.workspaces.len() > MAX_WORKSPACES
        || source
            .workspaces
            .windows(2)
            .any(|w| (&w[0].base, &w[0].name) >= (&w[1].base, &w[1].name))
    {
        return Err("FLAC移動履歴の元領域・件数・順序が不正です".into());
    }
    let mut claims = 0;
    let mut outputs = BTreeSet::new();
    for workspace in &source.workspaces {
        track_base(
            &workspace.base,
            workspace.base.rsplit('/').next().unwrap_or(""),
        )?;
        workspace.recorded.validate(true)?;
        workspace.observed.validate(true)?;
        if !matches!(workspace.name.as_str(), PLAN | PARTIAL_PLAN)
            || !inventory::valid_hash(&workspace.plan_hash)
            || workspace.tracks.len() > 2
        {
            return Err("FLAC移動履歴の計画が不正です".into());
        }
        if let Some(proof) = &workspace.recovery {
            if workspace.name != PARTIAL_PLAN {
                return Err("通常変換へ部分救出の入力証拠を適用しません".into());
            }
            proof.validate()?;
            claims += usize::from(proof.document.is_some());
        }
        claims += 1;
        for track in &workspace.tracks {
            let base = track_base(&workspace.base, &track.id)?;
            track.recorded.validate(true)?;
            track.observed.validate(true)?;
            if track.id == workspace.base.rsplit('/').next().unwrap_or("")
                || !outputs.insert(base.clone())
                || track.bodies.len() > 1024
            {
                return Err("FLAC移動履歴の出力先が重複・上限超過です".into());
            }
            claims += 1 + track.bodies.len();
            let mut ids = BTreeSet::new();
            for body in &track.bodies {
                body.recorded.validate(false)?;
                body.observed.validate(false)?;
                if !uuid(&body.id)
                    || !ids.insert(&body.id)
                    || !inventory::valid_hash(&body.checkpoint_hash)
                    || !inventory::valid_staging(&body.staging_name, &body.id)
                    || (body.path != format!("{base}/{}", body.staging_name)
                        && body.path != format!("{base}/{}.flac", body.id))
                {
                    return Err("FLAC移動履歴のチェックポイントが不正です".into());
                }
            }
        }
    }
    if claims > MAX_CLAIMS
        || serde_json::to_vec(source).map_err(|e| e.to_string())?.len() > MAX_BYTES
    {
        return Err("FLAC移動履歴が安全容量を超えています".into());
    }
    Ok(())
}
fn source_identity(snapshot: &NativeSnapshot<'_>, path: &str) -> Result<Identity, String> {
    Identity::read(
        &snapshot
            .source_metadata(path)?
            .ok_or("FLACの元の実体は移動対象ではありません")?,
    )
}

impl Source {
    pub(in crate::storage) fn capture(
        storage: &Storage,
        snapshot: &NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        storage.managed_anchor.verify()?;
        if storage.root != snapshot.source_root() {
            return Err("FLACの元領域と固定した移動元が一致しません".into());
        }
        let db = Connection::open_with_flags(
            snapshot.image().join("meetings.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA query_only=ON")
            .map_err(|e| e.to_string())?;
        let mut query = db.prepare("SELECT r.meeting_id,r.id FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE m.source_type='recording' AND r.input_history_snapshot_json IS NULL ORDER BY r.meeting_id,r.id")
            .map_err(|e| e.to_string())?;
        let mut workspaces = Vec::new();
        let native_files: BTreeMap<_, _> = snapshot
            .manifest()
            .entries
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry))
            .collect();
        let mut ids = BTreeSet::new();
        let mut count = 0;
        for row in query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
        {
            partial::check_cancel(cancelled)?;
            let (meeting, session) = row.map_err(|e| e.to_string())?;
            if !uuid(&meeting) || !uuid(&session) {
                continue;
            }
            let base = format!("meetings/{meeting}/audio/{session}");
            if snapshot.source_metadata(&base)?.is_none() {
                continue;
            }
            let root = Directory::open(&storage.root.join(&base))?;
            let Some(lock) = root.optional(LOCK, false).unwrap_or(None) else {
                continue;
            };
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                return Err("FLAC変換中は所有記録を移動しません".into());
            }
            for name in [PLAN, PARTIAL_PLAN] {
                let Some((bytes, stamp)) = root.read_json(name).unwrap_or(None) else {
                    continue;
                };
                let Ok(original) = serde_json::from_slice::<Plan>(&bytes) else {
                    continue;
                };
                let mut effective = original.clone();
                effective_plan(storage, &root.path, name, &bytes, &mut effective)?;
                if effective.recording_directory != root.identity {
                    continue;
                }
                if effective.schema_version != 1
                    || !inventory::valid_hash(&effective.input_hash)
                    || effective.tracks.is_empty()
                    || effective.tracks.len() > 2
                {
                    return Err("FLAC移動元の計画の形式・件数が不正です".into());
                }
                let plan_path = format!("{base}/{name}");
                if native_files
                    .get(plan_path.as_str())
                    .and_then(|e| e.hash.as_deref())
                    != Some(hash(&bytes).as_str())
                    || source_identity(snapshot, &base)? != root.identity
                {
                    return Err("FLACの固定した計画と移動元の実体が一致しません".into());
                }
                let mut tracks = Vec::new();
                for (raw, track) in original.tracks.iter().zip(&effective.tracks) {
                    partial::check_cancel(cancelled)?;
                    if !uuid(&track.id)
                        || !ids.insert(track.id.clone())
                        || track.id == session
                        || track.chunks.is_empty()
                        || track.chunks.len() > 1024
                    {
                        return Err("FLAC移動元の識別子・区間数が不正です".into());
                    }
                    let track_path = track_base(&base, &track.id)?;
                    let Some(_) = snapshot.source_metadata(&track_path)? else {
                        continue;
                    };
                    let directory = Directory::open(&storage.root.join(&track_path))?;
                    if directory.identity != track.directory {
                        continue;
                    }
                    let mut bodies = Vec::new();
                    for chunk in &track.chunks {
                        partial::check_cancel(cancelled)?;
                        if !uuid(chunk) || !ids.insert(chunk.clone()) {
                            return Err("FLAC移動元の区間IDが不正・重複しています".into());
                        }
                        let checkpoint_name = format!(".{chunk}.json");
                        let Some((bytes, stamp)) = directory.read_json(&checkpoint_name)? else {
                            continue;
                        };
                        let original: Checkpoint =
                            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                        let mut checkpoint = original.clone();
                        effective_checkpoint(
                            storage,
                            &directory.path,
                            &checkpoint_name,
                            &bytes,
                            &mut checkpoint,
                        )?;
                        if checkpoint.schema_version != 1
                            || checkpoint.input_hash != effective.input_hash
                            || checkpoint.chunk_id != *chunk
                            || !inventory::valid_staging(&checkpoint.staging_name, chunk)
                            || (checkpoint.staging_identity.is_none() && checkpoint.audio.is_some())
                        {
                            return Err("FLAC移動元のチェックポイントが計画と一致しません".into());
                        }
                        let staging = directory.optional(&checkpoint.staging_name, false)?;
                        let final_name = format!("{chunk}.flac");
                        let final_file = directory.optional(&final_name, false)?;
                        if staging.is_some() && final_file.is_some() {
                            return Err("FLAC移動元の途中音声と公開音声が重複しています".into());
                        }
                        if let (Some(recorded), Some(observed)) =
                            (&original.staging_identity, &checkpoint.staging_identity)
                        {
                            let file = staging
                                .map(|f| (checkpoint.staging_name.clone(), f))
                                .or_else(|| {
                                    final_file
                                        .filter(|_| checkpoint.audio.is_some())
                                        .map(|f| (final_name, f))
                                });
                            if let Some((name, file)) = file {
                                if Identity::file(&file)? == *observed {
                                    let path = format!("{track_path}/{name}");
                                    if source_identity(snapshot, &path)? != *observed {
                                        return Err("FLAC音声の元の所有実体が一致しません".into());
                                    }
                                    let checkpoint_path = format!("{track_path}/{checkpoint_name}");
                                    if native_files
                                        .get(checkpoint_path.as_str())
                                        .and_then(|e| e.hash.as_deref())
                                        != Some(hash(&bytes).as_str())
                                    {
                                        return Err(
                                            "FLACチェックポイントが固定した移動像と一致しません"
                                                .into(),
                                        );
                                    }
                                    bodies.push(Body {
                                        id: chunk.clone(),
                                        checkpoint_hash: hash(&bytes),
                                        staging_name: checkpoint.staging_name.clone(),
                                        path,
                                        recorded: recorded.clone(),
                                        observed: observed.clone(),
                                    });
                                }
                            }
                        }
                        stamp.require(&directory.required(&checkpoint_name, false)?)?;
                    }
                    directory.verify()?;
                    if source_identity(snapshot, &track_path)? != directory.identity {
                        return Err("FLAC出力先の元の所有実体が一致しません".into());
                    }
                    count += 1 + bodies.len();
                    tracks.push(TrackFacts {
                        id: track.id.clone(),
                        recorded: raw.directory.clone(),
                        observed: directory.identity,
                        bodies,
                    });
                }
                stamp.require(&root.required(name, false)?)?;
                let recovery = if name == PARTIAL_PLAN {
                    recovery::capture(storage, snapshot, &base, &original, &bytes, cancelled)?
                } else {
                    None
                };
                count += 1 + usize::from(recovery.as_ref().is_some_and(|p| p.document.is_some()));
                if count > MAX_CLAIMS || workspaces.len() >= MAX_WORKSPACES {
                    return Err("FLAC移動元の所有記録が安全上限を超えています".into());
                }
                workspaces.push(Workspace {
                    base: base.clone(),
                    name: name.into(),
                    plan_hash: hash(&bytes),
                    recovery,
                    recorded: original.recording_directory,
                    observed: root.identity.clone(),
                    tracks,
                });
            }
            root.verify()?;
        }
        if workspaces.is_empty() {
            return Ok(None);
        }
        workspaces.sort_by(|a, b| (&a.base, &a.name).cmp(&(&b.base, &b.name)));
        let source = SourceFacts {
            version: 1,
            source_root: storage.root.clone(),
            root: metadata(&storage.root, "")?,
            database: metadata(&storage.root, "meetings.sqlite3")?,
            native_hash: snapshot.manifest_hash().into(),
            workspaces,
        };
        valid_source(&source)?;
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        storage.managed_anchor.verify()?;
        Ok(Some(Self(source)))
    }

    pub(in crate::storage) fn bind_destination(
        &self,
        snapshot: &NativeSnapshot<'_>,
        placement: &Placement<'_>,
        operation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Destination, String> {
        valid_source(&self.0)?;
        snapshot.verify_source(cancelled)?;
        placement.verify(snapshot, cancelled)?;
        if self.0.source_root != snapshot.source_root()
            || self.0.native_hash != snapshot.manifest_hash()
            || !uuid(operation)
            || placement.progress()?.operation_id != operation
        {
            return Err("FLACの移動元と所有付き配置が一致しません".into());
        }
        let target = placement.progress()?.target;
        let copied = |path: &str, directory: bool| -> Result<Identity, String> {
            let identity = Identity::read(
                &placement
                    .owned_path(path)?
                    .ok_or("FLACの配置した所有対象がありません")?,
            )?;
            identity.validate(directory)?;
            Ok(identity)
        };
        let mut workspaces = Vec::new();
        for workspace in &self.0.workspaces {
            partial::check_cancel(cancelled)?;
            let mut tracks = Vec::new();
            for track in &workspace.tracks {
                let mut bodies = Vec::new();
                for body in &track.bodies {
                    partial::check_cancel(cancelled)?;
                    bodies.push(copied(&body.path, false)?);
                }
                tracks.push(BoundTrack {
                    id: track.id.clone(),
                    directory: copied(&track_base(&workspace.base, &track.id)?, true)?,
                    bodies,
                });
            }
            workspaces.push(BoundWorkspace {
                base: workspace.base.clone(),
                name: workspace.name.clone(),
                recording: copied(&workspace.base, true)?,
                tracks,
                recovery_document: workspace
                    .recovery
                    .as_ref()
                    .and_then(|p| p.document.as_ref())
                    .map(|_| copied(&format!("{}/partial-recovery.json", workspace.base), false))
                    .transpose()?,
            });
        }
        let destination = DestinationFacts {
            version: 1,
            operation: operation.into(),
            native_hash: self.0.native_hash.clone(),
            target_root: target.clone(),
            root: metadata(&target, "")?,
            database: copied("meetings.sqlite3", false)?,
            workspaces,
        };
        snapshot.verify_source(cancelled)?;
        placement.verify(snapshot, cancelled)?;
        Ok(Destination(destination))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    version: u32,
    native_manifest_hash: String,
    history: serde_json::Value,
    deletion_history: Option<serde_json::Value>,
    flac_history: Option<serde_json::Value>,
    excluded_area: String,
}
pub(in crate::storage) fn admitted_history(
    admission: &NativeHistoryAdmission<'_, '_>,
) -> Result<Option<Arc<Aliases>>, String> {
    admission.verify()?;
    let saved = admission.control()?;
    if saved.len() > 64 * 1024 * 1024 || hash(saved.as_bytes()) != admission.control_hash() {
        return Err("FLACの承認済み制御記録の容量・SHAが一致しません".into());
    }
    let control: Control = serde_json::from_str(&saved).map_err(|e| e.to_string())?;
    if control.native_manifest_hash != admission.native_hash()
        || !inventory::valid_hash(&control.native_manifest_hash)
        || !control.history.is_object()
        || control.excluded_area.is_empty()
        || control.excluded_area.len() > 255
        || control.version
            != if control.flac_history.is_some() {
                5
            } else if control.deletion_history.is_some() {
                4
            } else {
                3
            }
    {
        return Err("FLACの承認済み制御記録の形式・対象が不正です".into());
    }
    crate::storage::backup::relocation::control::verify_registry_image(
        admission.root(),
        &admission.root().join(".mon-scratch.sqlite3"),
        admission.operation(),
        admission.native_hash(),
        admission.control_hash(),
    )?;
    let root = Directory::open(admission.root())?;
    let registry_file = root.required(".mon-scratch.sqlite3", false)?;
    let registry_stamp = Stamp::read(&registry_file)?;
    let registry = Connection::open_with_flags(
        admission.root().join(".mon-scratch.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    registry
        .execute_batch("PRAGMA query_only=ON")
        .map_err(|e| e.to_string())?;
    let local_json: String = registry.query_row("SELECT local_json FROM relocation_history WHERE operation_id=?1 AND length(CAST(local_json AS BLOB))<=67108864", [admission.operation()], |r| r.get(0)).map_err(|e| e.to_string())?;
    let local: serde_json::Value = serde_json::from_str(&local_json).map_err(|e| e.to_string())?;
    let Some(facts) = control.flac_history else {
        if local.get("flac_history").is_some() || local.get("flac_binding").is_some() {
            return Err("FLAC履歴のない移動へ所有対応を追加しません".into());
        }
        registry_stamp.require(&root.required(".mon-scratch.sqlite3", false)?)?;
        admission.verify()?;
        return Ok(None);
    };
    if facts.is_null() || local.get("flac_history") != Some(&facts) {
        return Err("FLACの移動由来と元の制御記録が一致しません".into());
    }
    let source: SourceFacts = serde_json::from_value(facts).map_err(|e| e.to_string())?;
    valid_source(&source)?;
    let destination: DestinationFacts = serde_json::from_value(
        local
            .get("flac_binding")
            .ok_or("FLACのコピー先所有記録がありません")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    let (source_path, source_root, source_db) = admission.source();
    destination.root.validate(true)?;
    destination.database.validate(false)?;
    if source.source_root != source_path
        || source.root.registered()? != source_root
        || source.database.registered()? != source_db
        || source.native_hash != admission.native_hash()
        || destination.version != 1
        || destination.operation != admission.operation()
        || destination.native_hash != admission.native_hash()
        || destination.target_root != admission.root()
        || destination.root != root.identity
        || destination.database != metadata(admission.root(), "meetings.sqlite3")?
        || destination.database.registered()? != admission.owned_path("meetings.sqlite3", false)?
        || destination.workspaces.len() != source.workspaces.len()
    {
        return Err("FLACの承認した元領域・DBと移動先が一致しません".into());
    }
    let mut plans = BTreeMap::new();
    let mut checkpoints = BTreeMap::new();
    for (workspace, bound) in source.workspaces.into_iter().zip(destination.workspaces) {
        bound.recording.validate(true)?;
        if bound.base != workspace.base
            || bound.name != workspace.name
            || bound.tracks.len() != workspace.tracks.len()
            || bound.recording.registered()? != admission.owned_directory(&workspace.base)?
            || admission.copied_file_hash(&format!("{}/{}", workspace.base, workspace.name))?
                != workspace.plan_hash
        {
            return Err("FLAC計画の移動先は配置した所有対象ではありません".into());
        }
        recovery::admit(admission, &workspace, &bound)?;
        for (track, bound_track) in workspace.tracks.iter().zip(&bound.tracks) {
            bound_track.directory.validate(true)?;
            let track_path = track_base(&workspace.base, &track.id)?;
            if track.id != bound_track.id
                || track.bodies.len() != bound_track.bodies.len()
                || bound_track.directory.registered()? != admission.owned_directory(&track_path)?
            {
                return Err("FLAC出力先は配置した所有対象ではありません".into());
            }
            for (body, copied) in track.bodies.iter().zip(&bound_track.bodies) {
                copied.validate(false)?;
                let path = format!("{track_path}/.{}.json", body.id);
                if copied.registered()? != admission.owned_path(&body.path, false)?
                    || admission.copied_file_hash(&path)? != body.checkpoint_hash
                    || checkpoints
                        .insert(
                            path,
                            (
                                body.checkpoint_hash.clone(),
                                body.recorded.clone(),
                                copied.clone(),
                            ),
                        )
                        .is_some()
                {
                    return Err("FLAC音声は配置した実体・チェックポイントではありません".into());
                }
            }
        }
        plans.insert(
            (workspace.base.clone(), workspace.name.clone()),
            (workspace, bound),
        );
    }
    // Check both immutable control and the currently pinned origin again. New
    // local checkpoints may evolve later; these aliases only match old bytes.
    let after: String = registry
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            [admission.operation()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if after != local_json || admission.control()? != saved {
        return Err("FLAC履歴が承認中に変更されています".into());
    }
    registry_stamp.require(&root.required(".mon-scratch.sqlite3", false)?)?;
    admission.verify()?;
    Ok(Some(Arc::new(Aliases {
        root: admission.root().into(),
        root_identity: destination.root,
        database: destination.database,
        plans,
        checkpoints,
    })))
}
impl Aliases {
    fn verify(&self, storage: &Storage) -> Result<(), String> {
        storage.managed_anchor.verify()?;
        if storage.root != self.root
            || metadata(&self.root, "")? != self.root_identity
            || metadata(&self.root, "meetings.sqlite3")? != self.database
        {
            return Err("FLAC所有対応の承認済み保存先・DBが変更されています".into());
        }
        Ok(())
    }
}
pub(super) fn effective_plan(
    storage: &Storage,
    directory: &Path,
    name: &str,
    bytes: &[u8],
    plan: &mut Plan,
) -> Result<(), String> {
    let Some(aliases) = &storage.flac_history else {
        return Ok(());
    };
    aliases.verify(storage)?;
    let base = directory
        .strip_prefix(&storage.root)
        .map_err(|e| e.to_string())?
        .to_str()
        .ok_or("FLAC所有対応のパスが不正です")?;
    let Some((source, bound)) = aliases.plans.get(&(base.into(), name.into())) else {
        return Ok(());
    };
    if hash(bytes) != source.plan_hash {
        return Ok(());
    }
    if plan.recording_directory != source.recorded {
        return Err("FLAC計画の元の所有記録が移動履歴と一致しません".into());
    }
    plan.recording_directory = bound.recording.clone();
    for (source, bound) in source.tracks.iter().zip(&bound.tracks) {
        let track = plan
            .tracks
            .iter_mut()
            .find(|t| t.id == source.id)
            .ok_or("FLAC移動履歴にない出力計画です")?;
        if track.directory != source.recorded {
            return Err("FLAC出力計画の元の所有が一致しません".into());
        }
        track.directory = bound.directory.clone();
    }
    Ok(())
}
pub(super) fn effective_checkpoint(
    storage: &Storage,
    directory: &Path,
    name: &str,
    bytes: &[u8],
    checkpoint: &mut Checkpoint,
) -> Result<bool, String> {
    let Some(aliases) = &storage.flac_history else {
        return Ok(false);
    };
    aliases.verify(storage)?;
    let path = directory
        .join(name)
        .strip_prefix(&storage.root)
        .map_err(|e| e.to_string())?
        .to_str()
        .ok_or("FLACチェックポイントのパスが不正です")?
        .to_owned();
    let Some((sha, recorded, copied)) = aliases.checkpoints.get(&path) else {
        return Ok(false);
    };
    if hash(bytes) != *sha {
        return Ok(false);
    }
    if checkpoint.staging_identity.as_ref() != Some(recorded) {
        return Err("FLACチェックポイントの元の所有が一致しません".into());
    }
    checkpoint.staging_identity = Some(copied.clone());
    Ok(recorded != copied)
}
