//! Optional source cleanup is authorized only by the active local cutover.
//! A portable result contains no capability to remove files on this machine.
use super::*;
use crate::storage::deletion::salvage_source::{managed_audio, Target};
use mon_contracts::{SalvageSourceCleanup, SalvageSourceCleanupState as CleanupState};
use mon_native::identity::Proof;
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

const MANIFEST: &str = ".salvage-source-cleanup.json";
const PROGRESS: &str = ".salvage-source-cleanup-progress.json";
const PROGRESS_LIMIT: u64 = 4096;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    operation: String,
    source: OwnedDirectory,
    destination: Root,
    folder: OwnedDirectory,
    database_files: usize,
    targets: Vec<Target>,
}

impl Manifest {
    fn validate(&self, operation: &str, root: &Root) -> Result<(), String> {
        absolute(&self.source.path)?;
        if self.version != 1
            || self.operation != operation
            || self.destination != *root
            || self.destination.path.parent() != Some(self.folder.path.as_path())
            || overlaps(&self.source.path, &root.path)
            || self.source.identity == root.directory
            || !(1..=CONTROL.len()).contains(&self.database_files)
            || self.database_files > self.targets.len()
            || self
                .targets
                .first()
                .is_none_or(|t| t.relative != CONTROL[0])
        {
            return Err("元データの削除対象と復元先が一致しません".into());
        }
        let mut paths = BTreeSet::new();
        for (index, target) in self.targets.iter().enumerate() {
            target.validate(self.source.identity.device)?;
            if !paths.insert(target.relative.as_str())
                || if index < self.database_files {
                    !CONTROL.contains(&target.relative.as_str())
                } else {
                    !managed_audio(&target.relative)
                }
            {
                return Err("元の保存先で削除できるファイルを確認できませんでした".into());
            }
        }
        self.folder.verify()
    }

    fn view(&self, cursor: Option<&Cursor>) -> Result<SalvageSourceCleanup, String> {
        Ok(SalvageSourceCleanup {
            source_root: self
                .source
                .path
                .to_str()
                .ok_or("復元元を表示できません")?
                .into(),
            database_files: self.database_files.try_into().map_err(err)?,
            audio_files: (self.targets.len() - self.database_files)
                .try_into()
                .map_err(err)?,
            retained_audio_files: cursor
                .map(|cursor| cursor.retained_audio_files)
                .filter(|count| *count > 0),
            state: match cursor {
                None => CleanupState::Available,
                Some(c) if c.next == self.targets.len() => CleanupState::Completed,
                Some(_) => CleanupState::Interrupted,
            },
        })
    }

    fn preflight(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.verify_controls(0, false)?;
        for (index, target) in self.targets[..self.database_files].iter().enumerate() {
            check_cleanup_cancel(cancelled)?;
            let (parent, leaf) = target.parent(&self.source.path)?;
            if parent.observe(leaf)?.as_ref() != Some(&target.proof)
                || parent.observe(&self.reserved(index))?.is_some()
            {
                return Err("元の会議データが確認したときと異なるため、削除を始められませんでした".into());
            }
        }
        Ok(())
    }

    fn verify_controls(&self, next: usize, active: bool) -> Result<(), String> {
        for name in CONTROL {
            let position = self.targets[..self.database_files]
                .iter()
                .position(|target| target.relative == name);
            if active && position == Some(next) {
                continue;
            }
            let expected = position
                .filter(|index| *index >= next)
                .map(|index| &self.targets[index]);
            if Target::capture(&self.source.path, name)?.as_ref() != expected {
                return Err("元の会議データが確認したときと異なるため、削除を止めました".into());
            }
        }
        Ok(())
    }

    fn verify_model_boundary(&self, selection: &SelectionRecord) -> Result<(), String> {
        if overlaps(&self.source.path, &selection.models) {
            return Err("元の保存先と現在のモデルの保存先が重なっているため、削除できません".into());
        }
        self.folder.verify()?;
        match native_fs::symlink_metadata(&selection.models) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                && selection.models.parent() == Some(self.folder.path.as_path()) =>
            {
                // A missing sibling of the current meeting store cannot hold
                // model files. Keep the parent bound while checking absence.
                self.folder.verify()
            }
            _ => selection.verify_models(),
        }
    }

    fn reserved(&self, index: usize) -> String {
        format!(".mon-delete-salvage-{}-{index}", self.operation)
    }
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn check_cleanup_cancel(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        return Err("元データの削除を中断しました。復元結果から削除状態を確認してください".into());
    }
    Ok(())
}

fn same_observation(proof: &Proof, observed: &Observed) -> bool {
    proof.identity.registered().is_ok_and(|identity| {
        identity
            == (
                observed.identity.device,
                observed.identity.inode,
                observed.identity.birth_ns,
            )
    }) && proof.size == observed.len
        && (proof.modified_seconds, proof.modified_nanos) == observed.modified
        && (proof.change_seconds, proof.change_nanos) == observed.changed
}

pub(super) fn prepare(
    journal: &Journal,
    source: &OwnedDirectory,
    folder: &OwnedDirectory,
    destination: &Root,
    plan: &Plan,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<String>, String> {
    let Some(snapshot) = &journal.source_snapshot else {
        return Ok(None);
    };
    let mut targets = Vec::new();
    for (name, observed) in CONTROL.iter().zip(snapshot) {
        let Ok(target) = Target::capture(&source.path, name) else {
            // A copy can be rescued even when the original has different
            // ownership. Do not offer cleanup in that case.
            return Ok(None);
        };
        match (target, observed) {
            (Some(target), Some(observed)) if same_observation(&target.proof, observed) => {
                targets.push(target);
            }
            (None, None) => {}
            _ => return Ok(None),
        }
    }
    let database_files = targets.len();
    let mut seen = BTreeSet::new();
    for target in plan.copied_source_audio() {
        check_cancel(cancelled)?;
        if seen.insert(target.relative.as_str())
            && Target::capture(&source.path, &target.relative)
                .is_ok_and(|now| now.as_ref() == Some(target))
        {
            targets.push(target.clone());
        }
    }
    let manifest = Manifest {
        version: 1,
        operation: journal.operation_id.clone(),
        source: source.clone(),
        destination: destination.clone(),
        folder: folder.clone(),
        database_files,
        targets,
    };
    manifest.validate(&journal.operation_id, destination)?;
    let bytes = serde_json::to_vec(&manifest).map_err(err)?;
    if bytes.len() as u64 > PLAN_LIMIT {
        return Ok(None);
    }
    replace_owned_file(folder, MANIFEST, &bytes)?;
    Ok(Some(hash(&bytes)))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum Boundary {
    Armed,
    Staged(Proof),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    manifest: String,
    next: usize,
    boundary: Option<Boundary>,
    #[serde(default)]
    retained_audio_files: u32,
}

impl Cursor {
    fn read(manifest: &Manifest, digest: &str) -> Result<Option<Self>, String> {
        manifest.folder.verify()?;
        let path = manifest.folder.path.join(PROGRESS);
        if !exists(&path)? {
            return Ok(None);
        }
        let value: Self =
            serde_json::from_slice(&reset::read_private(&path, PROGRESS_LIMIT)?).map_err(err)?;
        if value.manifest != digest
            || value.next > manifest.targets.len()
            || (value.next == manifest.targets.len() && value.boundary.is_some())
            || value.retained_audio_files as usize > value.next.saturating_sub(manifest.database_files)
        {
            return Err("元データの削除進捗が対象と一致しません".into());
        }
        if let Some(Boundary::Staged(ref proof)) = value.boundary {
            proof.validate(false)?;
            if !proof.matches_after_rename(&manifest.targets[value.next].proof) {
                return Err("削除途中のファイルが確認したときと異なるため、削除を続けられません".into());
            }
        }
        Ok(Some(value))
    }

    fn save(&self, manifest: &Manifest, previous: Option<&Self>) -> Result<(), String> {
        let folder = &manifest.folder;
        folder.verify()?;
        let temporary = folder
            .path
            .join(format!(".salvage-cleanup-{}.tmp", Uuid::new_v4()));
        reset::write_new(&temporary, &serde_json::to_vec(self).map_err(err)?)?;
        let result = (|| {
            folder.verify()?;
            if Self::read(manifest, &self.manifest)?.as_ref() != previous {
                return Err("元データの削除進捗が変更されています".into());
            }
            reset::rename(&temporary, &folder.path.join(PROGRESS), previous.is_some())
        })();
        if result.is_err() {
            let expected = serde_json::to_vec(self).map_err(err)?;
            remove_matching_file(&temporary, |actual| actual == expected)?;
        }
        result
    }

    fn transition(
        &mut self,
        manifest: &Manifest,
        update: impl FnOnce(&mut Self),
    ) -> Result<(), String> {
        let mut next = self.clone();
        update(&mut next);
        next.save(manifest, Some(self))?;
        *self = next;
        Ok(())
    }

    fn retain_audio(&mut self, manifest: &Manifest) -> Result<(), String> {
        self.transition(manifest, |next| {
            next.next += 1;
            next.boundary = None;
            // The manifest's audio count already fits in the public u32 view.
            next.retained_audio_files += 1;
        })
    }
}

impl Bootstrap {
    fn source_cleanup_manifest(
        &self,
        state: &State,
        operation_id: &str,
    ) -> Result<Option<(Manifest, String)>, String> {
        let Some(record) = state.loaded.as_ref().map(|loaded| &loaded.record) else {
            return Ok(None);
        };
        let Some(selection) = &record.salvage else {
            return Ok(None);
        };
        let Some(digest) = &selection.source_cleanup_sha256 else {
            return Ok(None);
        };
        if selection.operation_id != operation_id
            || record.origin != Origin::Salvaged
            || record.pending_move.is_some()
            || self.restart_required.load(Ordering::Acquire)
            || self.selected != record.current.path
            || self.models_root != selection.models
        {
            return Ok(None);
        }
        self.guard(state)?;
        record.current.verify()?;
        let parent = record
            .current
            .path
            .parent()
            .ok_or("復元先の親がありません")?;
        let bytes = reset::read_private(&parent.join(MANIFEST), PLAN_LIMIT)?;
        if hash(&bytes) != *digest {
            return Err("元データの削除対象が変更されています".into());
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(err)?;
        manifest.validate(operation_id, &record.current)?;
        manifest.verify_model_boundary(selection)?;
        Ok(Some((manifest, digest.clone())))
    }

    pub(crate) fn salvage_source_cleanup(
        &self,
        operation_id: &str,
    ) -> Result<Option<SalvageSourceCleanup>, String> {
        let state = self.state.lock().map_err(err)?;
        let Some((manifest, digest)) = self.source_cleanup_manifest(&state, operation_id)? else {
            return Ok(None);
        };
        let cursor = Cursor::read(&manifest, &digest)?;
        Ok(Some(manifest.view(cursor.as_ref())?))
    }

    pub(crate) fn delete_salvage_source(
        &self,
        operation_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<SalvageSourceCleanup, String> {
        let state = self.state.lock().map_err(err)?;
        let (manifest, digest) = self
            .source_cleanup_manifest(&state, operation_id)?
            .ok_or("現在の復元先に対応する元データの削除記録がありません")?;
        let existing = Cursor::read(&manifest, &digest)?;
        if existing
            .as_ref()
            .is_some_and(|cursor| cursor.next == manifest.targets.len())
        {
            return manifest.view(existing.as_ref());
        }
        check_cleanup_cancel(cancelled)?;
        manifest.source.verify()?;
        let lease = reset::RootLease::acquire_for_salvage(&manifest.source.path)?;
        let selection = state
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.record.salvage.as_ref())
            .ok_or("現在の復元先を確認できません")?;
        let guard = || {
            self.guard(&state)?;
            manifest.destination.verify()?;
            manifest.folder.verify()?;
            manifest.source.verify()?;
            manifest.verify_model_boundary(selection)?;
            lease.verify(&manifest.source.path)
        };
        let mut cursor = if let Some(existing) = existing {
            existing
        } else {
            crate::operation_observation::stage(
                "checking_salvage_source",
                Some(manifest.database_files as u64),
                "files",
            );
            manifest.preflight(cancelled)?;
            check_cleanup_cancel(cancelled)?;
            guard()?;
            let consent = Cursor {
                manifest: digest,
                next: 0,
                boundary: None,
                retained_audio_files: 0,
            };
            consent.save(&manifest, None)?;
            consent
        };
        crate::operation_observation::report(
            "deleting_salvage_source",
            cursor.next as u64,
            manifest.targets.len() as u64,
            "files",
        );
        while let Some(target) = manifest.targets.get(cursor.next) {
            check_cleanup_cancel(cancelled)?;
            guard()?;
            manifest.verify_controls(cursor.next, cursor.boundary.is_some())?;
            if cursor.next >= manifest.database_files && !can_remove_audio(&manifest, &cursor, target) {
                cursor.retain_audio(&manifest)?;
            } else {
                remove_target(&manifest, &mut cursor, target)?;
            }
            crate::operation_observation::advance(1);
        }
        manifest.view(Some(&cursor))
    }
}

fn can_remove_audio(manifest: &Manifest, cursor: &Cursor, target: &Target) -> bool {
    let observe = || {
        let (parent, leaf) = target.parent(&manifest.source.path)?;
        Ok::<_, String>((parent.observe(leaf)?, parent.observe(&manifest.reserved(cursor.next))?))
    };
    let Ok((original, reserved)) = observe() else {
        return false;
    };
    match (&cursor.boundary, original, reserved) {
        (None | Some(Boundary::Armed), Some(original), None) => original == target.proof,
        (Some(Boundary::Armed), None, Some(staged)) => staged.matches_after_rename(&target.proof),
        (Some(Boundary::Staged(expected)), None, Some(staged)) => expected == &staged,
        (Some(Boundary::Staged(_)), None, None) => true,
        _ => false,
    }
}

fn remove_target(manifest: &Manifest, cursor: &mut Cursor, target: &Target) -> Result<(), String> {
    let (parent, leaf) = target.parent(&manifest.source.path)?;
    let reserved = manifest.reserved(cursor.next);
    if cursor.boundary.is_none() {
        if parent.observe(leaf)?.as_ref() != Some(&target.proof)
            || parent.observe(&reserved)?.is_some()
        {
            return Err("元のファイルが確認したときと異なるため、削除を止めました".into());
        }
        cursor.transition(manifest, |next| next.boundary = Some(Boundary::Armed))?;
        cleanup_checkpoint("armed")?;
    }
    if cursor.boundary == Some(Boundary::Armed) {
        let renamed = match (parent.observe(leaf)?, parent.observe(&reserved)?) {
            (Some(original), None) if original == target.proof => {
                parent.reserve(leaf, &reserved)?;
                cleanup_checkpoint("reserved")?;
                true
            }
            (None, Some(staged)) if staged.matches_after_rename(&target.proof) => {
                parent.sync()?;
                false
            }
            _ => return Err("削除途中のファイルを確認できないため、削除を止めました".into()),
        };
        let staged = match parent.observe(&reserved) {
            Ok(Some(staged)) if staged.matches_after_rename(&target.proof) => staged,
            other => {
                if renamed {
                    // A raced replacement is never unlinked. Move it back only
                    // when the original name is still vacant.
                    let _ = parent.reserve(&reserved, leaf);
                }
                return Err(other
                    .err()
                    .unwrap_or_else(|| "削除途中のファイルが変わったため、削除を止めました".into()));
            }
        };
        cursor.transition(manifest, |next| {
            next.boundary = Some(Boundary::Staged(staged))
        })?;
        cleanup_checkpoint("staged")?;
    }
    let Some(Boundary::Staged(staged)) = &cursor.boundary else {
        return Err("どのファイルまで削除したか確認できないため、削除を続けられません".into());
    };
    if parent.observe(leaf)?.is_some() {
        return Err("元の名前に別のファイルが作成されています。削除を停止しました".into());
    }
    match parent.observe(&reserved)? {
        Some(actual) if &actual == staged => parent.remove(&reserved, staged)?,
        None => parent.sync()?,
        Some(_) => return Err("削除途中のファイルが変わったため、削除を続けられません".into()),
    }
    cleanup_checkpoint("removed")?;
    cursor.transition(manifest, |next| {
        next.next += 1;
        next.boundary = None;
    })
}

#[cfg(not(test))]
fn cleanup_checkpoint(_: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static INTERRUPT_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn cleanup_checkpoint(phase: &str) -> Result<(), String> {
    if INTERRUPT_AT.get() == Some(phase) {
        Err("simulated source cleanup interruption".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed(base: &Path) -> (PathBuf, PathBuf, PathBuf, String) {
        let (config, app_data, source) = super::super::tests::source(base);
        std::fs::write(source.join("unrelated.wav"), b"user file").unwrap();
        std::fs::create_dir_all(app_data.join("models")).unwrap();
        std::fs::write(app_data.join("models/unrelated"), b"old models").unwrap();
        let id = rescue(base, &config, &app_data, &source);
        (config, app_data, source, id)
    }

    fn rescue(base: &Path, config: &Path, app_data: &Path, source: &Path) -> String {
        let destination = base.join("destination");
        reset::create_directory(&destination).unwrap();
        let mut session = Session::open(config, app_data).unwrap();
        assert!(session
            .b
            .salvage_source_cleanup(&session.journal.operation_id)
            .unwrap()
            .is_none());
        session.select(destination, false).unwrap();
        let id = session
            .inspect(&|| false)
            .unwrap()
            .preview
            .unwrap()
            .operation_id;
        session.commit(&id, &|| false).unwrap();
        assert!(source.join(CONTROL[0]).is_file());
        session.leave().unwrap();
        assert!(source.join(CONTROL[0]).is_file());
        id
    }

    #[test]
    fn source_cleanup_is_explicit_local_and_resumes_every_removal_boundary() {
        for checkpoint in ["armed", "reserved", "staged", "removed"] {
            let temp = tempfile::tempdir().unwrap();
            let (config, app_data, source, id) = committed(&temp.path().canonicalize().unwrap());
            let b = Bootstrap::open(&config, &app_data).unwrap();
            let ready = b.salvage_source_cleanup(&id).unwrap().unwrap();
            assert_eq!(ready.state, CleanupState::Available);
            assert!(ready.database_files >= 1);
            assert_eq!(ready.audio_files, 0);
            assert!(b
                .salvage_source_cleanup(&Uuid::new_v4().to_string())
                .unwrap()
                .is_none());
            INTERRUPT_AT.set(Some(checkpoint));
            let interrupted = b.delete_salvage_source(&id, &|| false);
            INTERRUPT_AT.set(None);
            assert!(interrupted
                .err()
                .is_some_and(|error| error.contains("simulated")));
            drop(b);

            let b = Bootstrap::open(&config, &app_data).unwrap();
            assert_eq!(
                b.salvage_source_cleanup(&id).unwrap().unwrap().state,
                CleanupState::Interrupted
            );
            let storage = b.open_storage().unwrap();
            assert!(storage.has_current_salvage_result(&id).unwrap());
            let finished = b.delete_salvage_source(&id, &|| false).unwrap();
            assert_eq!(finished.state, CleanupState::Completed);
            assert_eq!(b.delete_salvage_source(&id, &|| false).unwrap(), finished);
            assert!(CONTROL.iter().all(|name| !source.join(name).exists()));
            assert!(source.join(".mon-startup-reset.lock").exists());
            assert_eq!(
                std::fs::read(source.join("unrelated.wav")).unwrap(),
                b"user file"
            );
            assert_eq!(
                std::fs::read(app_data.join("models/unrelated")).unwrap(),
                b"old models"
            );
            assert_eq!(storage.list_meetings().unwrap().len(), 1);
        }
    }

    #[test]
    fn source_cleanup_removes_copied_managed_audio_and_keeps_unregistered_audio() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let config = base.join("config");
        let app_data = base.join("application");
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let storage = b.open_storage().unwrap();
        let source = storage.root().to_owned();
        let relative = salvage::seed_cleanup_audio_fixture(&storage, "source-cleanup-audio");
        assert!(managed_audio(&relative), "{relative}");
        let original = source.join(relative);
        let retained = original.with_file_name(format!("{}.wav", Uuid::new_v4()));
        std::fs::copy(&original, &retained).unwrap();
        storage.keep_sqlite_controls_on_close(true).unwrap();
        b.request_salvage().unwrap();
        drop(storage);
        drop(b);

        let id = rescue(&base, &config, &app_data, &source);
        assert!(original.is_file());
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let storage = b.open_storage().unwrap();
        let status = b.salvage_source_cleanup(&id).unwrap().unwrap();
        assert_eq!(status.audio_files, 1);
        assert_eq!(
            b.delete_salvage_source(&id, &|| false).unwrap().state,
            CleanupState::Completed,
        );
        assert!(!original.exists());
        assert!(retained.is_file());
        let meetings = storage.list_meetings().unwrap();
        assert_eq!(meetings.len(), 1);
        assert!(storage.canonical_audio_plan(&meetings[0].id).is_ok());
    }

    #[test]
    fn source_cleanup_preserves_changed_audio_and_continues_after_interruption() {
        for interrupted in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let config = base.join("config");
            let app_data = base.join("application");
            let b = Bootstrap::open(&config, &app_data).unwrap();
            let storage = b.open_storage().unwrap();
            let source = storage.root().to_owned();
            for operation in ["source-cleanup-first", "source-cleanup-next"] {
                salvage::seed_cleanup_audio_fixture(&storage, operation);
            }
            storage.keep_sqlite_controls_on_close(true).unwrap();
            b.request_salvage().unwrap();
            drop(storage);
            drop(b);

            let id = rescue(&base, &config, &app_data, &source);
            let b = Bootstrap::open(&config, &app_data).unwrap();
            let (manifest, _) = b
                .source_cleanup_manifest(&b.state.lock().unwrap(), &id)
                .unwrap()
                .unwrap();
            let expected = &manifest.targets[manifest.database_files..];
            assert_eq!(expected.len(), 2);
            let changed = source.join(&expected[0].relative);
            let unchanged = source.join(&expected[1].relative);
            if interrupted {
                let cancelled = || CONTROL.iter().all(|name| !source.join(name).exists());
                assert!(b.delete_salvage_source(&id, &cancelled).is_err());
                let status = b.salvage_source_cleanup(&id).unwrap().unwrap();
                assert_eq!(status.state, CleanupState::Interrupted);
                assert_eq!(status.retained_audio_files, None);
            }
            std::fs::write(&changed, b"changed since restoration").unwrap();
            drop(b);

            let b = Bootstrap::open(&config, &app_data).unwrap();
            let status = b.delete_salvage_source(&id, &|| false).unwrap();
            assert_eq!(status.state, CleanupState::Completed);
            assert_eq!(status.retained_audio_files, Some(1));
            assert_eq!(std::fs::read(changed).unwrap(), b"changed since restoration");
            assert!(!unchanged.exists());
            assert!(CONTROL.iter().all(|name| !source.join(name).exists()));
            assert_eq!(b.delete_salvage_source(&id, &|| false).unwrap(), status);
        }
    }

    #[test]
    fn source_cleanup_allows_missing_model_sibling_but_rejects_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let (config, app_data, source, id) = committed(&temp.path().canonicalize().unwrap());
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let models = b.models_root.clone();
        let moved = models.with_file_name("moved-models");
        std::fs::rename(&models, &moved).unwrap();
        reset::create_directory(&models).unwrap();
        assert!(b.delete_salvage_source(&id, &|| false).is_err());
        assert!(source.join(CONTROL[0]).exists());
        std::fs::remove_dir(&models).unwrap();
        let status = b.delete_salvage_source(&id, &|| false).unwrap();
        assert_eq!(status.state, CleanupState::Completed);
        assert!(!source.join(CONTROL[0]).exists());
        assert!(moved.is_dir());
    }

    #[test]
    fn source_cleanup_keeps_all_originals_if_preflight_finds_a_changed_database() {
        let temp = tempfile::tempdir().unwrap();
        let (config, app_data, source, id) = committed(&temp.path().canonicalize().unwrap());
        let original = source.join(CONTROL[0]);
        let retained = source.join("manual-backup.sqlite3");
        std::fs::rename(&original, &retained).unwrap();
        std::fs::write(&original, b"replacement").unwrap();
        let b = Bootstrap::open(&config, &app_data).unwrap();
        assert!(b.delete_salvage_source(&id, &|| false).is_err());
        assert_eq!(
            b.salvage_source_cleanup(&id).unwrap().unwrap().state,
            CleanupState::Available
        );
        assert_eq!(std::fs::read(original).unwrap(), b"replacement");
        assert!(retained.is_file());
    }
}
