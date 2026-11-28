//! Physical correspondence for an unfinished single-WAV reservation. Only a
//! verified native snapshot and its owned placement can mint these facts.
use super::NativeSnapshot;
use crate::bootstrap::placement::{NativeHistoryAdmission, Placement};
use crate::storage::publication_files::{Directory, Identity, Stamp};
use crate::storage::Storage;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_CLAIMS: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;
type FileIdentity = (u64, u64, i64);

fn hash(bytes: &[u8]) -> String {
    mon_core::hash::sha256_hex(bytes)
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identity(root: &Path, path: &str) -> Result<Identity, String> {
    Identity::read(&crate::native_fs::symlink_metadata(root.join(path)).map_err(|e| e.to_string())?)
}
fn file_identity(value: &Identity) -> Result<FileIdentity, String> {
    let (device, inode, birth) = value.registered()?;
    Ok((
        device,
        inode,
        i64::try_from(birth).map_err(|_| "WAVの実体時刻が範囲外です")?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    owner: String,
    base: String,
    reservation_hash: String,
    recorded: FileIdentity,
    observed: Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFacts {
    version: u32,
    source_root: PathBuf,
    root: Identity,
    database: Identity,
    native_hash: String,
    claims: Vec<Claim>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(in crate::storage) struct Source(SourceFacts);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bound {
    owner: String,
    observed: Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationFacts {
    version: u32,
    operation: String,
    target_root: PathBuf,
    root: Identity,
    database: Identity,
    native_hash: String,
    claims: Vec<Bound>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(in crate::storage) struct Destination(DestinationFacts);

// No Deserialize: serialized content cannot construct an admitted alias.
pub(crate) struct Aliases {
    root: PathBuf,
    root_identity: Identity,
    database: Identity,
    claims: BTreeMap<String, (Claim, Identity)>,
}
fn validate(source: &SourceFacts) -> Result<(), String> {
    source.root.validate(true)?;
    source.database.validate(false)?;
    if source.version != 1
        || !source.source_root.is_absolute()
        || !sha(&source.native_hash)
        || source.claims.is_empty()
        || source.claims.len() > MAX_CLAIMS
        || source
            .claims
            .windows(2)
            .any(|pair| pair[0].owner >= pair[1].owner)
        || serde_json::to_vec(source).map_err(|e| e.to_string())?.len() > MAX_BYTES
    {
        return Err("WAVの移動由来の範囲・形式が不正です".into());
    }
    for claim in &source.claims {
        let parts: Vec<_> = claim.base.split('/').collect();
        if !uuid(&claim.owner)
            || parts.len() != 4
            || parts[0] != "meetings"
            || parts[2] != "audio"
            || !uuid(parts[1])
            || parts[3] != claim.owner
            || !sha(&claim.reservation_hash)
            || claim.recorded.2 < 0
        {
            return Err("WAVの移動由来の所属が不正です".into());
        }
        claim.observed.validate(false)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Reservation {
    schema_version: u32,
    owner_id: String,
    file_name: String,
    device: u64,
    inode: u64,
    birth_nanoseconds: i64,
}
impl Source {
    pub(in crate::storage) fn capture(
        storage: &Storage,
        snapshot: &NativeSnapshot<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        snapshot.verify(cancelled)?;
        snapshot.verify_source(cancelled)?;
        if storage.root != snapshot.source_root() {
            return Err("WAVの固定した移動元が一致しません".into());
        }
        let db = Connection::open_with_flags(
            snapshot.image().join("meetings.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA query_only=ON")
            .map_err(|e| e.to_string())?;
        let sessions = {
            let mut query = db.prepare("SELECT r.meeting_id,r.id FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE m.source_type='recording' AND NOT EXISTS(SELECT 1 FROM meeting_audio_commits a WHERE a.meeting_id=r.meeting_id) ORDER BY r.id").map_err(|e| e.to_string())?;
            let result = query
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        };
        let mut claims = Vec::new();
        for (meeting, owner) in sessions {
            if cancelled() {
                return Err("WAVの移動由来の検査を中断しました".into());
            }
            if !uuid(&meeting) || !uuid(&owner) {
                return Err("WAV録音の識別子が不正です".into());
            }
            if crate::storage::deletion::audio::source_deleted(&db, &meeting)? {
                continue;
            }
            let base = format!("meetings/{meeting}/audio/{owner}");
            let reservation_path = format!("{base}/single-wav-owner.json");
            if snapshot.source_metadata(&reservation_path)?.is_none() {
                continue;
            }
            let directory = Directory::open(&storage.root.join(&base))?;
            let (bytes, stamp) = directory
                .read_json("single-wav-owner.json")?
                .ok_or("WAVの所有予約が消失しました")?;
            if bytes.len() > 4096 {
                return Err("WAVの所有予約が大きすぎます".into());
            }
            let reservation: Reservation =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if reservation.schema_version != 1
                || reservation.owner_id != owner
                || reservation.file_name != "recording.wav"
                || reservation.birth_nanoseconds < 0
            {
                return Err("WAVの移動元の所有予約が不正です".into());
            }
            let reservation_hash = hash(&bytes);
            if !snapshot.manifest().entries.iter().any(|entry| {
                entry.relative_path == reservation_path
                    && entry.hash.as_deref() == Some(&reservation_hash)
            }) {
                return Err("WAVの所有予約は固定した移動対象ではありません".into());
            }
            let file = directory.required("recording.wav", false)?;
            let observed = Identity::file(&file)?;
            let path = format!("{base}/recording.wav");
            let expected = Identity::read(
                &snapshot
                    .source_metadata(&path)?
                    .ok_or("WAVの元の実体がありません")?,
            )?;
            if observed != expected {
                return Err("WAVの元の実体が変更されています".into());
            }
            let recorded = (
                reservation.device,
                reservation.inode,
                reservation.birth_nanoseconds,
            );
            let effective = storage
                .wav_history
                .as_ref()
                .map(|aliases| aliases.effective_identity(&owner, &reservation_hash, recorded))
                .transpose()?
                .flatten()
                .unwrap_or(recorded);
            if effective != file_identity(&observed)? {
                return Err("WAVの所有予約と元の実体が一致しません".into());
            }
            stamp.require(&directory.required("single-wav-owner.json", false)?)?;
            claims.push(Claim {
                owner,
                base,
                reservation_hash,
                recorded,
                observed,
            });
            if claims.len() > MAX_CLAIMS {
                return Err("WAV移動対象が安全件数を超えています".into());
            }
        }
        if claims.is_empty() {
            return Ok(None);
        }
        let source = SourceFacts {
            version: 1,
            source_root: storage.root.clone(),
            root: identity(&storage.root, "")?,
            database: identity(&storage.root, "meetings.sqlite3")?,
            native_hash: snapshot.manifest_hash().into(),
            claims,
        };
        validate(&source)?;
        snapshot.verify_source(cancelled)?;
        Ok(Some(Self(source)))
    }
    pub(in crate::storage) fn bind_destination(
        &self,
        snapshot: &NativeSnapshot<'_>,
        placement: &Placement<'_>,
        operation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Destination, String> {
        validate(&self.0)?;
        snapshot.verify_source(cancelled)?;
        placement.verify(snapshot, cancelled)?;
        let progress = placement.progress()?;
        if self.0.source_root != snapshot.source_root()
            || self.0.native_hash != snapshot.manifest_hash()
            || !uuid(operation)
            || progress.operation_id != operation
        {
            return Err("WAVの移動元と所有付き配置が一致しません".into());
        }
        let copied = |path: &str| -> Result<Identity, String> {
            Identity::read(
                &placement
                    .owned_path(path)?
                    .ok_or("配置したWAVの実体がありません")?,
            )
        };
        let mut claims = Vec::new();
        for claim in &self.0.claims {
            let observed = copied(&format!("{}/recording.wav", claim.base))?;
            observed.validate(false)?;
            claims.push(Bound {
                owner: claim.owner.clone(),
                observed,
            });
        }
        let result = DestinationFacts {
            version: 1,
            operation: operation.into(),
            target_root: progress.target.clone(),
            root: identity(&progress.target, "")?,
            database: copied("meetings.sqlite3")?,
            native_hash: self.0.native_hash.clone(),
            claims,
        };
        snapshot.verify_source(cancelled)?;
        placement.verify(snapshot, cancelled)?;
        Ok(Destination(result))
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
    wav_history: Option<serde_json::Value>,
    excluded_area: String,
}
pub(in crate::storage) fn admitted_history(
    admission: &NativeHistoryAdmission<'_, '_>,
) -> Result<Option<Arc<Aliases>>, String> {
    admission.verify()?;
    let saved = admission.control()?;
    if saved.len() > 64 * 1024 * 1024 || hash(saved.as_bytes()) != admission.control_hash() {
        return Err("WAVの承認済み制御記録が不正です".into());
    }
    let control: Control = serde_json::from_str(&saved).map_err(|e| e.to_string())?;
    let expected = if control.wav_history.is_some() {
        6
    } else if control.flac_history.is_some() {
        5
    } else if control.deletion_history.is_some() {
        4
    } else {
        3
    };
    if control.version != expected
        || control.native_manifest_hash != admission.native_hash()
        || !control.history.is_object()
        || control.excluded_area.is_empty()
        || control.excluded_area.len() > 255
    {
        return Err("WAVの承認済み制御記録の所属が不正です".into());
    }
    super::control::verify_registry_image(
        admission.root(),
        &admission.root().join(".mon-scratch.sqlite3"),
        admission.operation(),
        admission.native_hash(),
        admission.control_hash(),
    )?;
    let root = Directory::open(admission.root())?;
    let registry_file = root.required(".mon-scratch.sqlite3", false)?;
    let stamp = Stamp::read(&registry_file)?;
    let registry = Connection::open_with_flags(
        admission.root().join(".mon-scratch.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|e| e.to_string())?;
    registry
        .execute_batch("PRAGMA query_only=ON")
        .map_err(|e| e.to_string())?;
    let local_json:String=registry.query_row("SELECT local_json FROM relocation_history WHERE operation_id=?1 AND length(CAST(local_json AS BLOB))<=67108864",[admission.operation()],|row|row.get(0)).map_err(|e|e.to_string())?;
    let local: serde_json::Value = serde_json::from_str(&local_json).map_err(|e| e.to_string())?;
    let Some(facts) = control.wav_history else {
        if local.get("wav_history").is_some() || local.get("wav_binding").is_some() {
            return Err("WAV履歴のない移動へ実体対応を追加しません".into());
        }
        stamp.require(&root.required(".mon-scratch.sqlite3", false)?)?;
        admission.verify()?;
        return Ok(None);
    };
    if facts.is_null() || local.get("wav_history") != Some(&facts) {
        return Err("WAVの移動由来が一致しません".into());
    }
    let source: SourceFacts = serde_json::from_value(facts).map_err(|e| e.to_string())?;
    validate(&source)?;
    let destination: DestinationFacts = serde_json::from_value(
        local
            .get("wav_binding")
            .ok_or("WAVのコピー先所有記録がありません")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    #[cfg(all(test, target_os = "macos"))]
    let destination = DestinationFacts {
        root: destination.root.perturb_relocation_identity("wav"),
        ..destination
    };
    let (source_path, source_root, source_db) = admission.source();
    let database = identity(admission.root(), "meetings.sqlite3")?;
    if source.source_root != source_path
        || source.root.registered()? != source_root
        || source.database.registered()? != source_db
        || source.native_hash != admission.native_hash()
        || destination.version != 1
        || destination.operation != admission.operation()
        || destination.native_hash != admission.native_hash()
        || destination.target_root != admission.root()
        || !destination.root.matches_relocation_history(&root.identity)
        || !destination.database.matches_relocation_history(&database)
        || destination.database.registered()? != admission.owned_path("meetings.sqlite3", false)?
        || destination.claims.len() != source.claims.len()
    {
        return Err("WAVの承認した保存元・DBと配置先が一致しません".into());
    }
    destination.root.validate(true)?;
    destination.database.validate(false)?;
    let mut claims = BTreeMap::new();
    for (claim, mut bound) in source.claims.into_iter().zip(destination.claims) {
        bound.observed.validate(false)?;
        if bound.owner != claim.owner
            || bound.observed.registered()?
                != admission.owned_path(&format!("{}/recording.wav", claim.base), false)?
            || admission.copied_file_hash(&format!("{}/single-wav-owner.json", claim.base))?
                != claim.reservation_hash
        {
            return Err("WAVの所有対応は検証済みコピーの実体ではありません".into());
        }
        bound
            .observed
            .rebase_relocation_device(&destination.root, &root.identity);
        claims.insert(claim.owner.clone(), (claim, bound.observed));
    }
    let after: String = registry
        .query_row(
            "SELECT local_json FROM relocation_history WHERE operation_id=?1",
            [admission.operation()],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if after != local_json || admission.control()? != saved {
        return Err("WAVの移動由来が承認中に変更されています".into());
    }
    stamp.require(&root.required(".mon-scratch.sqlite3", false)?)?;
    admission.verify()?;
    root.verify()?;
    if identity(admission.root(), "meetings.sqlite3")? != database {
        return Err("WAV履歴の承認中に会議DBが変更されました".into());
    }
    Ok(Some(Arc::new(Aliases {
        root: admission.root().into(),
        root_identity: root.identity,
        database,
        claims,
    })))
}
impl Aliases {
    pub(crate) fn effective_identity(
        &self,
        owner: &str,
        reservation_sha256: &str,
        source: FileIdentity,
    ) -> Result<Option<FileIdentity>, String> {
        let current = identity(&self.root, "")?;
        #[cfg(all(test, target_os = "macos"))]
        let current = current.perturb_relocation_identity("wav-live");
        if current != self.root_identity
            || identity(&self.root, "meetings.sqlite3")? != self.database
        {
            return Err("WAVの承認済み保存先・DBが変更されています".into());
        }
        let Some((claim, copied)) = self.claims.get(owner) else {
            return Ok(None);
        };
        if claim.reservation_hash != reservation_sha256 || claim.recorded != source {
            return Err("WAVの移動元の所有予約が変更されています".into());
        }
        let directory = Directory::open(&self.root.join(&claim.base))?;
        copied.require(&directory.required("recording.wav", false)?)?;
        Ok(Some(file_identity(copied)?))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::bootstrap::Bootstrap;
    use std::io::Write;
    use std::os::unix::fs::DirBuilderExt;

    fn pending(storage: &Storage) -> (String, String, String, FileIdentity) {
        let meeting = storage.create_meeting("単一WAVの未完了録音").unwrap().id;
        let owner = uuid::Uuid::new_v4().to_string();
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE meetings SET source_type='recording',lifecycle='recording' WHERE id=?1",
                [&meeting],
            )
            .unwrap();
            db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,created_at,updated_at) VALUES(?1,?2,'interrupted','system','{}',?1,0,0)",rusqlite::params![owner,meeting]).unwrap();
        }
        let base = format!("meetings/{meeting}/audio/{owner}");
        let path = storage.root.join(&base);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .unwrap();
        let mut file = super::super::private_file(&path.join("recording.wav")).unwrap();
        file.write_all(&mon_core::capture::wav::header(160).unwrap())
            .unwrap();
        file.write_all(&[0; 320]).unwrap();
        file.sync_all().unwrap();
        let recorded = file_identity(&Identity::file(&file).unwrap()).unwrap();
        let bytes=serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"ownerId":owner,"fileName":"recording.wav","device":recorded.0,"inode":recorded.1,"birthNanoseconds":recorded.2})).unwrap();
        let mut reservation =
            super::super::private_file(&path.join("single-wav-owner.json")).unwrap();
        reservation.write_all(&bytes).unwrap();
        reservation.sync_all().unwrap();
        (owner, base, hash(&bytes), recorded)
    }
    fn move_storage(storage: Storage, config: &Path, legacy: &Path, target: &Path) -> Storage {
        if !config.join("bootstrap.json").exists() {
            crate::bootstrap::set_test_root(config, legacy, storage.root());
        }
        let bootstrap = Bootstrap::open(config, legacy).unwrap();
        bootstrap.initialize(&storage).unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(target)
            .unwrap();
        let snapshot = storage.prepare_relocation_snapshot(&|| false).unwrap();
        let revision = bootstrap.view().unwrap().revision;
        let operation = uuid::Uuid::new_v4().to_string();
        let confirmation = uuid::Uuid::new_v4().to_string();
        bootstrap
            .begin_move(
                &operation,
                &confirmation,
                target,
                snapshot.manifest_hash(),
                revision,
            )
            .unwrap();
        let mut placement = bootstrap.pending_placement(&operation, &snapshot).unwrap();
        placement.copy(&snapshot, &|| false).unwrap();
        let handoff = snapshot.prepare_control_handoff(&|| false).unwrap();
        let control: serde_json::Value = serde_json::from_slice(handoff.bytes()).unwrap();
        assert_eq!(control["version"], 6);
        assert_eq!(
            control["wav_history"]["claims"].as_array().unwrap().len(),
            1
        );
        placement.install_history(&handoff, &|| false).unwrap();
        placement
            .activate_for_restart(&handoff, &confirmation, revision + 1, &|| false)
            .unwrap();
        drop(placement);
        drop(handoff);
        drop(snapshot);
        drop(storage);
        drop(bootstrap);
        let bootstrap = Bootstrap::open(config, legacy).unwrap();
        assert_eq!(bootstrap.root(), target);
        bootstrap.open_storage().unwrap()
    }
    #[test]
    fn wav_relocation_startup_accepts_only_a_saved_device_difference() {
        use crate::test_support::relocation_identity::with_change;

        let parent = tempfile::tempdir_in("/private/tmp").unwrap();
        let storage = Storage::open(parent.path().join("source")).unwrap();
        let (owner, base, reservation_hash, recorded) = pending(&storage);
        let config = parent.path().join("config");
        let legacy = parent.path().join("legacy");
        let target = parent.path().join("target");
        let reservation =
            std::fs::read(storage.root.join(&base).join("single-wav-owner.json")).unwrap();
        let storage = with_change("wav", true, || {
            move_storage(storage, &config, &legacy, &target)
        });
        assert!(!storage.data_write_blocked());
        let aliases = storage.wav_history.as_ref().unwrap();
        assert!(aliases
            .effective_identity(&owner, &reservation_hash, recorded)
            .unwrap()
            .is_some());
        assert!(with_change("wav-live", true, || aliases.effective_identity(
            &owner,
            &reservation_hash,
            recorded
        ))
        .is_err());
        assert_eq!(
            std::fs::read(target.join(&base).join("single-wav-owner.json")).unwrap(),
            reservation
        );
        drop(storage);

        let bootstrap = Bootstrap::open(&config, &legacy).unwrap();
        let error = with_change("wav", false, || bootstrap.open_storage())
            .err()
            .expect("inode が違う WAV 履歴を受け付けない");
        assert!(error.contains("WAVの承認した保存元・DBと配置先が一致しません"));
    }

    #[test]
    fn pending_single_wav_relocation_keeps_original_reservation_across_two_cold_moves() {
        let parent = tempfile::tempdir_in("/private/tmp").unwrap();
        let storage = Storage::open(parent.path().join("source")).unwrap();
        let (owner, base, reservation_hash, recorded) = pending(&storage);
        let original =
            std::fs::read(storage.root.join(&base).join("single-wav-owner.json")).unwrap();
        let config = parent.path().join("config");
        let legacy = parent.path().join("legacy");
        let first = move_storage(storage, &config, &legacy, &parent.path().join("first"));
        let copied = first
            .wav_history
            .as_ref()
            .unwrap()
            .effective_identity(&owner, &reservation_hash, recorded)
            .unwrap()
            .unwrap();
        assert_ne!(copied, recorded);
        assert_eq!(
            std::fs::read(first.root.join(&base).join("single-wav-owner.json")).unwrap(),
            original
        );
        let second = move_storage(first, &config, &legacy, &parent.path().join("second"));
        let aliases = second.wav_history.as_ref().unwrap();
        let current = aliases
            .effective_identity(&owner, &reservation_hash, recorded)
            .unwrap()
            .unwrap();
        assert_ne!(current, copied);
        assert_ne!(current, recorded);
        assert!(aliases
            .effective_identity(&owner, &"a".repeat(64), recorded)
            .is_err());
        assert!(aliases
            .effective_identity(
                &owner,
                &reservation_hash,
                (recorded.0, recorded.1 + 1, recorded.2)
            )
            .is_err());
        let path = second.root.join(&base).join("recording.wav");
        let body = std::fs::read(&path).unwrap();
        std::fs::rename(&path, path.with_extension("original")).unwrap();
        let mut replacement = super::super::private_file(&path).unwrap();
        replacement.write_all(&body).unwrap();
        replacement.sync_all().unwrap();
        assert!(
            aliases
                .effective_identity(&owner, &reservation_hash, recorded)
                .is_err(),
            "same PCM bytes cannot authorize another physical object"
        );
    }
}
