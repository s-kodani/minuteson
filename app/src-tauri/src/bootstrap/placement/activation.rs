//! Selection precedes first-launch admission. Only the owned placement journal
//! records admission; bootstrap contains no user-supplied "verified" flag.
use super::*;
use crate::storage::backup::relocation::control::ControlHandoff;
use std::sync::atomic::Ordering;

pub(super) const ADMISSION_SCHEMA: &str = "
CREATE TABLE admission(id INTEGER PRIMARY KEY CHECK(id=1),root TEXT NOT NULL CHECK(length(CAST(root AS BLOB))<=16384),root_hash TEXT NOT NULL CHECK(length(root_hash)=64),intent_hash TEXT NOT NULL CHECK(length(intent_hash)=64),manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),control_hash TEXT NOT NULL CHECK(length(control_hash)=64),registry_identity TEXT NOT NULL CHECK(length(registry_identity)<=512),registry_hash TEXT NOT NULL CHECK(length(registry_hash)=64));
CREATE TRIGGER admission_fixed BEFORE UPDATE ON admission BEGIN SELECT RAISE(ABORT,'activation admission is immutable'); END;
CREATE TRIGGER admission_retained BEFORE DELETE ON admission BEGIN SELECT RAISE(ABORT,'activation admission is retained'); END;
";

// Borrowed from a live, OS-locked placement owner and the selected bootstrap
// record. JSON cannot construct this capability. It authorizes history reads
// at this root only; it grants neither original consent nor an unlink permit.
pub(crate) struct NativeHistoryAdmission<'a, 'b> {
    placement: &'a Placement<'b>,
    root: Root,
    activation: Activation,
}
impl NativeHistoryAdmission<'_, '_> {
    pub(crate) fn verify(&self) -> Result<(), String> {
        let state = self
            .placement
            .bootstrap
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.placement.bootstrap.guard(&state)?;
        let record = &state
            .loaded
            .as_ref()
            .ok_or("承認済み起動設定がありません")?
            .record;
        if record.current != self.root || record.activation.as_ref() != Some(&self.activation) {
            return Err("選択中の保存先と削除履歴の承認が一致しません".into());
        }
        drop(state);
        if !self.placement.admitted(&self.root, &self.activation)? {
            return Err("削除履歴を読み戻す配置先は未承認です".into());
        }
        Ok(())
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root.path
    }
    pub(crate) fn read_snapshot(&self) -> Result<crate::storage::read_cache::Snapshot, String> {
        self.verify()?;
        let snapshot = crate::storage::read_cache::Snapshot::create(
            &self.placement.bootstrap.read_cache_root()?,
        )?;
        self.verify()?;
        Ok(snapshot)
    }
    pub(crate) fn models_root(&self) -> &Path {
        &self.placement.bootstrap.models_root
    }
    pub(crate) fn operation(&self) -> &str {
        &self.activation.pending.operation_id
    }
    pub(crate) fn native_hash(&self) -> &str {
        &self.activation.pending.manifest_hash
    }
    pub(crate) fn control_hash(&self) -> &str {
        &self.activation.control_hash
    }
    pub(crate) fn source(&self) -> (&Path, (u64, u64, u64), (u64, u64, u64)) {
        let source = &self.activation.pending.source;
        (
            &source.path,
            (
                source.directory.device,
                source.directory.inode,
                source.directory.birth_ns,
            ),
            (
                source.database.device,
                source.database.inode,
                source.database.birth_ns,
            ),
        )
    }
    pub(crate) fn control(&self) -> Result<String, String> {
        self.verify()?;
        history::admitted_control(self.placement, self.control_hash())
    }
    pub(crate) fn owned_directory(&self, path: &str) -> Result<(u64, u64, u64), String> {
        self.owned_path(path, true)
    }
    pub(crate) fn copied_file_hash(&self, path: &str) -> Result<&str, String> {
        self.placement.guard()?;
        let node = self
            .placement
            .nodes
            .get(path)
            .ok_or("履歴の記録は配置対象ではありません")?;
        if node.entry.directory || node.state != "copied" {
            return Err("履歴の記録の配置が未完了です".into());
        }
        node.entry
            .hash
            .as_deref()
            .ok_or("履歴の記録のSHAがありません".into())
    }
    pub(crate) fn owned_path(
        &self,
        path: &str,
        directory: bool,
    ) -> Result<(u64, u64, u64), String> {
        // The complete selected admission is checked at both parser boundaries.
        // Keep each inventory lookup tied to this live owner without rescanning
        // the whole destination registry once per directory.
        self.placement.guard()?;
        let node = self
            .placement
            .nodes
            .get(path)
            .ok_or("履歴の親は配置台帳の所有対象ではありません")?;
        if node.entry.directory != directory || node.state != "copied" {
            return Err("履歴の親の配置が未完了です".into());
        }
        let id = node
            .identity
            .as_ref()
            .ok_or("履歴の親の所有実体がありません")?;
        Ok((id.device, id.inode, id.birth_ns))
    }
}

pub(super) fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.version != 1
        || !sha(&m.schema_hash)
        || !sha(&m.database_hash)
        || m.database_size < 100
        || m.database_size > i64::MAX as u64
        || m.entries.len() > 200_000
        || m.assets.len() > 200_000
        || m.tables.is_empty()
        || m.tables.len() > 1024
        || m.tables.windows(2).any(|t| t[0].name >= t[1].name)
        || m.tables.iter().any(|t| {
            t.name.is_empty() || t.name.len() > 1024 || t.rows > 2_000_000 || !sha(&t.hash)
        })
        || m.assets.windows(2).any(|a| a[0].id >= a[1].id)
    {
        return Err("保存済み移動manifestの形式・件数・SHAが不正です".into());
    }
    let mut entries = BTreeMap::new();
    for e in &m.entries {
        if e.relative_path.is_empty()
            || e.relative_path.len() > 1024
            || e.relative_path.contains(['\\', '\0'])
            || e.relative_path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || !matches!(
                e.relative_path.split('/').next(),
                Some("meetings" | "temp" | "voices")
            )
            || e.size > i64::MAX as u64
            || (e.directory && (e.size != 0 || e.hash.is_some()))
            || (!e.directory && !e.hash.as_ref().is_some_and(|h| sha(h)))
            || entries.insert(e.relative_path.as_str(), e).is_some()
        {
            return Err("保存済み移動manifestの管理パスが不正です".into());
        }
        if let Some((parent, _)) = e.relative_path.rsplit_once('/') {
            if !entries.get(parent).is_some_and(|p| p.directory) {
                return Err("保存済み移動manifestの親の所有がありません".into());
            }
        } else if !e.directory {
            return Err("移動対象の管理ルートはフォルダーである必要があります".into());
        }
    }
    if m.entries
        .windows(2)
        .any(|e| e[0].relative_path >= e[1].relative_path)
    {
        return Err("保存済み移動manifestの順序が不正です".into());
    }
    for a in &m.assets {
        let parts: Vec<_> = a.relative_path.split('/').collect();
        if a.id.is_empty()
            || a.id.len() > 1024
            || a.meeting_id.len() > 1024
            || parts.len() < 3
            || if a.meeting_id.is_empty() {
                crate::storage::voice::audio::relative(parts[1], &a.id).as_deref()
                    != Ok(a.relative_path.as_str())
                    || a.state != "ready"
            } else {
                parts[0] != "meetings" || parts[1] != a.meeting_id
            }
            || a.relative_path.len() > 1024
            || a.relative_path.contains(['\\', '\0'])
            || parts
                .iter()
                .any(|p| p.is_empty() || *p == "." || *p == "..")
            || a.size > i64::MAX as u64
            || !sha(&a.hash)
            || !matches!(
                a.state.as_str(),
                "ready" | "missing" | "corrupt" | "delete_pending"
            )
        {
            return Err("保存済み移動manifestの管理参照が不正です".into());
        }
    }
    Ok(())
}

impl Placement<'_> {
    fn verify_target(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.guard()?;
        if self.progress()?.state != "ready" {
            return Err("配置が完了していない保存先は切り替えません".into());
        }
        for node in self.nodes.values() {
            check(cancelled)?;
            if node.state != "copied" {
                return Err("配置した本文の所有確認が未完了です".into());
            }
            self.node_path(node)?;
        }
        verify_placed_image_with_controls(
            &self.pending.target,
            &self.manifest,
            cancelled,
            &[".mon-handoff.sqlite3", ".mon-scratch.sqlite3"],
        )?;
        self.guard()
    }

    fn admitted(&self, root: &Root, a: &Activation) -> Result<bool, String> {
        self.guard()?;
        let count: i64 = self
            .db
            .query_row("SELECT count(*) FROM admission", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(false);
        }
        let (json,rh,intent,manifest,control,registry,registry_hash): (String,String,String,String,String,String,String) = self.db.query_row("SELECT root,root_hash,intent_hash,manifest_hash,control_hash,registry_identity,registry_hash FROM admission WHERE id=1 AND length(CAST(root AS BLOB))<=16384 AND length(registry_identity)<=512", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|e|e.to_string())?;
        if count != 1
            || json != serde_json::to_string(root).map_err(|e| e.to_string())?
            || hash(json.as_bytes()) != rh
            || intent != hash(&serde_json::to_vec(&a.pending).map_err(|e| e.to_string())?)
            || manifest != self.pending.manifest_hash
            || control != a.control_hash
        {
            return Err("初回起動の承認対象が一致しません。空DBへ切り替えません".into());
        }
        let db_node = self
            .nodes
            .get("meetings.sqlite3")
            .ok_or("配置したDBの所有証拠がありません")?;
        if db_node.state != "copied"
            || db_node.identity.as_ref() != Some(&root.database)
            || root.directory != self.pending.target_directory
            || root.path != self.pending.target
        {
            return Err("承認済みルートと配置したDBの所有証拠が一致しません".into());
        }
        history::admitted_registry(self, &control, &read_identity(&registry, &registry_hash)?)?;
        root.verify()?;
        self.guard()?;
        Ok(true)
    }

    fn admit(&mut self, root: &Root, a: &Activation) -> Result<(), String> {
        if !self.admitted(root, a)? {
            let (_, control, registry) = history::installed_control(self)?;
            if control != a.control_hash {
                return Err("初回起動の履歴確認が一致しません".into());
            }
            self.verify_target(&|| false)?;
            root.verify()?;
            let db_node = self
                .nodes
                .get("meetings.sqlite3")
                .ok_or("配置したDBの所有証拠がありません")?;
            if db_node.identity.as_ref() != Some(&root.database) {
                return Err("切替対象DBの所有者が一致しません".into());
            }
            let json = serde_json::to_string(root).map_err(|e| e.to_string())?;
            let registry = serde_json::to_string(&registry).map_err(|e| e.to_string())?;
            self.guard()?;
            self.db
                .execute(
                    "INSERT INTO admission VALUES(1,?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        json,
                        hash(json.as_bytes()),
                        hash(&serde_json::to_vec(&a.pending).map_err(|e| e.to_string())?),
                        self.pending.manifest_hash,
                        control,
                        registry,
                        hash(registry.as_bytes())
                    ],
                )
                .map_err(|e| e.to_string())?;
            self.committed()?;
        }
        // Even a previously uncertain admission commit must be durable before
        // normal startup writes. Admission authorizes mutations, not a claim
        // that the original copied bytes remain forever unchanged.
        crate::native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&self.bootstrap.directory).map_err(|e| e.to_string())?;
        if !self.admitted(root, a)? {
            return Err("初回起動の承認結果が欠落しています".into());
        }
        Ok(())
    }

    pub(crate) fn activate_for_restart(
        &mut self,
        handoff: &ControlHandoff<'_, '_>,
        confirmation: &str,
        expected_revision: u64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<View, String> {
        self.guard()?;
        check(cancelled)?;
        if confirmation != self.pending.confirmation_id {
            return Err("保存先切替の確認対象が一致しません".into());
        }
        let mut record = {
            let state = self
                .bootstrap
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.bootstrap.guard(&state)?;
            state
                .loaded
                .as_ref()
                .ok_or("起動設定が未確定です")?
                .record
                .clone()
        };
        if let Some(a) = &record.activation {
            if a.pending == self.pending
                && record.pending_move.is_none()
                && expected_revision.checked_add(1) == Some(record.revision)
            {
                // Lost-response replay never republishes or touches the old
                // source. Legacy shared config roots may already have changed.
                if !self.admitted(&record.current, a)? {
                    let (_, control, _) = history::installed_control(self)?;
                    if control != a.control_hash {
                        return Err("切替結果の履歴が一致しません".into());
                    }
                    self.verify_target(cancelled)?;
                }
                return self.bootstrap.view();
            }
        }
        self.require_pending()?;
        if record.revision != expected_revision
            || record.pending_move.as_ref() != Some(&self.pending)
        {
            return Err("保存先切替の確認が古くなっています".into());
        }
        handoff.verify(cancelled)?;
        self.verify(handoff.snapshot(), cancelled)?;
        let (control_json, control_hash, _) = history::installed_control(self)?;
        handoff.check_compatible(control_json.as_bytes())?;
        self.inspect_deletion_handoff(handoff, cancelled)?
            .require_clean()?;
        // Only the held source, owned placement and freshly checked typed
        // histories can reach selection. The read-only inspection is not a
        // deletion permit; cold startup separately verifies live admission.
        self.verify_target(cancelled)?;
        let target = Root::inspect(&self.pending.target)?;
        if target.directory != self.pending.target_directory {
            return Err("切替先の実体が一致しません".into());
        }
        record.revision = record
            .revision
            .checked_add(1)
            .ok_or("起動設定の版が上限です")?;
        record.origin = Origin::Relocated;
        record.current = target.clone();
        record.pending_move = None;
        record.activation = Some(Activation {
            pending: self.pending.clone(),
            control_hash,
        });
        record.validate()?;
        // This irreversible fence is established before ANY config publication
        // attempt. Every later error requires restart; it cannot resume writes.
        handoff.snapshot().retire_source_for_restart(cancelled)?;
        self.bootstrap
            .restart_required
            .store(true, Ordering::Release);
        handoff.verify(&|| false)?;
        let (_, installed_hash, _) = history::installed_control(self)?;
        if record
            .activation
            .as_ref()
            .is_none_or(|a| a.control_hash != installed_hash)
        {
            return Err("保存先切替直前に履歴台帳が変更されています。書込停止を維持します".into());
        }
        self.verify_target(&|| false)?;
        target.verify()?;
        activation_checkpoint("before_selection");
        fault("before_selection")?;
        let mut state = self
            .bootstrap
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.bootstrap.guard(&state)?;
        let before = &state.loaded.as_ref().ok_or("起動設定が未確定です")?.record;
        if before.revision != expected_revision
            || before.pending_move.as_ref() != Some(&self.pending)
        {
            return Err("保存先切替直前に確認が失効しました。書込停止を維持します".into());
        }
        self.bootstrap.write(&mut state, record)?;
        drop(state);
        activation_checkpoint("after_selection");
        self.bootstrap.view()
    }
}

impl Bootstrap {
    pub(crate) fn open_storage(&self) -> Result<Storage, String> {
        let record = {
            let state = self
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.guard(&state)?;
            state.loaded.as_ref().map(|l| l.record.clone())
        };
        let cache_root = self.read_cache_root()?;
        crate::storage::read_cache::recover(&cache_root)?;
        {
            let state = self
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.guard(&state)?;
        }
        if let Some(recovery) = record
            .as_ref()
            .filter(|record| record.origin == Origin::Recovered)
            .and_then(|record| record.recovery.as_ref())
        {
            let storage = self.open_recovered_storage(recovery)?;
            self.initialize(&storage)?;
            return Ok(storage);
        }
        let storage = match record.and_then(|record| record.activation.map(|a| (record.current, a)))
        {
            None => Storage::open_with_models(self.root().to_owned(), self.models_root.clone())
                .map_err(|e| e.to_string())?,
            Some((root, activation)) => {
                let p = self.open_placement(activation.pending.clone(), None)?;
                let admission = NativeHistoryAdmission {
                    placement: &p,
                    root,
                    activation,
                };
                admission.verify()?;
                Storage::open_with_history(&admission)?
            }
        };
        self.initialize(&storage)?;
        Ok(storage)
    }
    pub(in crate::bootstrap) fn admit_activation(&self) -> Result<(), String> {
        let record = {
            let state = self
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.guard(&state)?;
            state.loaded.as_ref().map(|l| l.record.clone())
        };
        let Some(record) = record else {
            return Ok(());
        };
        let Some(a) = &record.activation else {
            return Ok(());
        };
        let mut p = self.open_placement(a.pending.clone(), None)?;
        p.admit(&record.current, a)?;
        activation_checkpoint("after_admission");
        Ok(())
    }
}

#[cfg(not(test))]
fn activation_checkpoint(_: &str) {}
#[cfg(not(test))]
fn fault(_: &str) -> Result<(), String> {
    Ok(())
}
pub(in crate::bootstrap) fn publication_checkpoint() -> Result<(), String> {
    fault("after_rename")
}
#[cfg(test)]
std::thread_local! { static FAULT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
fn fault(name: &str) -> Result<(), String> {
    if FAULT.with(|f| f.get() == Some(name)) {
        Err(format!("injected activation failure: {name}"))
    } else {
        Ok(())
    }
}
#[cfg(test)]
fn activation_checkpoint(name: &str) {
    if std::env::var("MON_ACTIVATION_TEST_CHECKPOINT").as_deref() == Ok(name) {
        println!("MON_ACTIVATION_READY={name}");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
