//! Destination-local durable ownership. The enclosing placement OS lease is
//! held throughout. Never initialize an existing empty/unproven journal.
use super::*;
use crate::storage::backup::relocation::control::{verify_registry_image, ControlHandoff};

const JOURNAL: &str = ".mon-handoff.sqlite3";
const REGISTRY: &str = ".mon-scratch.sqlite3";
const SCHEMA:&str="
CREATE TABLE handoff(id INTEGER PRIMARY KEY CHECK(id=1),intent TEXT NOT NULL CHECK(length(CAST(intent AS BLOB))<=65536),intent_hash TEXT NOT NULL CHECK(length(intent_hash)=64),journal_identity TEXT NOT NULL CHECK(length(journal_identity)<=512),journal_hash TEXT NOT NULL CHECK(length(journal_hash)=64),registry_identity TEXT CHECK(length(registry_identity)<=512),registry_hash TEXT CHECK(length(registry_hash)=64),control_json TEXT CHECK(length(CAST(control_json AS BLOB))<=67108864),control_hash TEXT CHECK(length(control_hash)=64),image_size INTEGER CHECK(image_size>=0),image_hash TEXT CHECK(length(image_hash)=64),state TEXT NOT NULL CHECK(state IN ('pending','owned','installed')),CHECK((state='pending')=(registry_identity IS NULL)),CHECK((registry_identity IS NULL)=(registry_hash IS NULL)),CHECK((control_json IS NULL)=(control_hash IS NULL)),CHECK((image_size IS NULL)=(image_hash IS NULL)),CHECK(state!='installed' OR (control_json IS NOT NULL AND image_hash IS NOT NULL)));
CREATE TRIGGER handoff_fixed BEFORE UPDATE ON handoff WHEN NEW.id!=OLD.id OR NEW.intent!=OLD.intent OR NEW.intent_hash!=OLD.intent_hash OR NEW.journal_identity!=OLD.journal_identity OR NEW.journal_hash!=OLD.journal_hash OR (OLD.registry_identity IS NOT NULL AND (NEW.registry_identity IS NOT OLD.registry_identity OR NEW.registry_hash IS NOT OLD.registry_hash)) OR OLD.state='installed' OR (OLD.state='owned' AND NEW.state='pending') BEGIN SELECT RAISE(ABORT,'handoff ownership is monotonic'); END;
CREATE TRIGGER handoff_retained BEFORE DELETE ON handoff BEGIN SELECT RAISE(ABORT,'handoff ownership is retained'); END;
";
struct Local {
    path: PathBuf,
    file: File,
    proof: ControlProof,
    db: Connection,
    expected_schema: Vec<(String, String, String)>,
    intent: String,
}
struct Saved {
    state: String,
    registry: Option<Identity>,
    control: Option<(String, String)>,
    image: Option<(u64, String)>,
}
impl Local {
    fn open(p: &Placement<'_>, create: bool) -> Result<Option<Self>, String> {
        p.guard()?;
        let path = p.pending.target.join(JOURNAL);
        let new = match crate::native_fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.to_string()),
            Ok(_) => false,
        };
        if new && !create {
            return Ok(None);
        }
        let base = cstring(JOURNAL)?;
        let file = descriptor(unsafe {
            libc::openat(
                p.root.as_raw_fd(),
                base.as_ptr(),
                libc::O_RDWR
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC
                    | if new { libc::O_CREAT | libc::O_EXCL } else { 0 },
                0o600,
            )
        })?;
        let initial =
            ControlProof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
        if !new && initial.size == 0 {
            return Err("未確定の移行先台帳を再初期化しません".into());
        }
        if new {
            crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
            crate::native_fs::sync_handle(&p.root).map_err(|e| e.to_string())?;
        }
        let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
        expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let expected_schema = schema(&expected)?;
        let db = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        if !initial
            .identity
            .matches(&crate::native_fs::symlink_metadata(&path).map_err(|e| e.to_string())?)
        {
            return Err("移行先台帳が開く途中で差し替えられています".into());
        }
        if !new
            && (schema(&db)? != expected_schema
                || db
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .map_err(|e| e.to_string())?
                    != 1)
        {
            return Err("移行先台帳の形式・版が不正です".into());
        }
        db.busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        let intent = serde_json::to_string(&p.pending).map_err(|e| e.to_string())?;
        if !new {
            let (count,saved,sha,j,h):(i64,String,String,String,String)=db.query_row("SELECT (SELECT count(*) FROM handoff),intent,intent_hash,journal_identity,journal_hash FROM handoff WHERE id=1 AND length(CAST(intent AS BLOB))<=65536 AND length(journal_identity)<=512",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
            if count != 1
                || saved != intent
                || hash(saved.as_bytes()) != sha
                || read_identity(&j, &h)? != initial.identity
                || db
                    .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    != "delete"
            {
                return Err("既存の移行先台帳の所有・確認対象が一致しません。変更しません".into());
            }
        }
        // The enclosing placement lease supplies process exclusion. NORMAL
        // mode removes rollback sidecars after each FULL-committed transaction.
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA locking_mode=NORMAL; BEGIN IMMEDIATE; COMMIT;").map_err(|e|e.to_string())?;
        if new {
            let identity = serde_json::to_string(&initial.identity).map_err(|e| e.to_string())?;
            db.execute_batch("BEGIN IMMEDIATE;")
                .map_err(|e| e.to_string())?;
            db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            db.execute(
                "INSERT INTO handoff VALUES(1,?1,?2,?3,?4,NULL,NULL,NULL,NULL,NULL,NULL,'pending')",
                params![
                    intent,
                    hash(intent.as_bytes()),
                    identity,
                    hash(identity.as_bytes())
                ],
            )
            .map_err(|e| e.to_string())?;
            db.execute_batch("PRAGMA user_version=1; COMMIT;")
                .map_err(|e| e.to_string())?;
            crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
            crate::native_fs::sync_handle(&p.root).map_err(|e| e.to_string())?;
        }
        let proof =
            ControlProof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
        if proof.identity != initial.identity {
            return Err("移行先台帳の実体が変更されています".into());
        }
        let local = Self {
            path,
            file,
            proof,
            db,
            expected_schema,
            intent,
        };
        local.guard(p)?;
        local.saved()?;
        Ok(Some(local))
    }
    fn guard(&self, p: &Placement<'_>) -> Result<(), String> {
        p.guard()?;
        if ControlProof::read(&crate::native_fs::metadata(&self.file).map_err(|e| e.to_string())?)?
            != self.proof
            || ControlProof::read(
                &crate::native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?,
            )? != self.proof
            || schema(&self.db)? != self.expected_schema
        {
            return Err("移行先台帳の実体・内容・形式が変更されています".into());
        }
        Ok(())
    }
    fn saved(&self) -> Result<Saved, String> {
        let (count,intent,sha,j,h):(i64,String,String,String,String)=self.db.query_row("SELECT (SELECT count(*) FROM handoff),intent,intent_hash,journal_identity,journal_hash FROM handoff WHERE id=1 AND length(CAST(intent AS BLOB))<=65536 AND length(journal_identity)<=512",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
        if count != 1
            || intent != self.intent
            || hash(intent.as_bytes()) != sha
            || read_identity(&j, &h)? != self.proof.identity
        {
            return Err("移行先台帳の所有・確認対象が一致しません".into());
        }
        let integrity: String = self
            .db
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err("移行先台帳が破損しています".into());
        }
        let (state,j,h,c,ch,n,ih):(String,Option<String>,Option<String>,Option<String>,Option<String>,Option<i64>,Option<String>)=self.db.query_row("SELECT state,registry_identity,registry_hash,control_json,control_hash,image_size,image_hash FROM handoff WHERE id=1 AND (control_json IS NULL OR length(CAST(control_json AS BLOB))<=67108864)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|e|e.to_string())?;
        let registry = match (j, h) {
            (Some(j), Some(h)) => Some(read_identity(&j, &h)?),
            (None, None) => None,
            _ => return Err("移行先台帳の所有証拠が欠損しています".into()),
        };
        let control = match (c, ch) {
            (Some(c), Some(h)) if hash(c.as_bytes()) == h => Some((c, h)),
            (None, None) => None,
            _ => return Err("移行先台帳の制御記録が変更されています".into()),
        };
        let image = match (n, ih) {
            (Some(n), Some(h)) if n >= 0 && h.len() == 64 => Some((n as u64, h)),
            (None, None) => None,
            _ => return Err("移行先台帳のコピー証拠が欠損しています".into()),
        };
        if !matches!(state.as_str(), "pending" | "owned" | "installed")
            || (state == "pending") != registry.is_none()
            || state == "installed" && (control.is_none() || image.is_none())
        {
            return Err("移行先台帳の進行状態が不正です".into());
        }
        Ok(Saved {
            state,
            registry,
            control,
            image,
        })
    }
    fn committed(&mut self, p: &Placement<'_>) -> Result<(), String> {
        crate::native_fs::sync_handle(&self.file).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&p.root).map_err(|e| e.to_string())?;
        let proof = ControlProof::read(
            &crate::native_fs::metadata(&self.file).map_err(|e| e.to_string())?,
        )?;
        if proof.identity != self.proof.identity {
            return Err("移行先台帳の同期中に実体が変更されています".into());
        }
        self.proof = proof;
        self.guard(p)
    }
    fn registry_file(&self, p: &Placement<'_>, saved: &Saved) -> Result<File, String> {
        self.guard(p)?;
        let id = saved
            .registry
            .as_ref()
            .ok_or("移行先台帳の本文は未登録です")?;
        let base = cstring(REGISTRY)?;
        let file = descriptor(unsafe {
            libc::openat(
                p.root.as_raw_fd(),
                base.as_ptr(),
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0,
            )
        })?;
        let fd = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
        let path = crate::native_fs::symlink_metadata(p.pending.target.join(REGISTRY))
            .map_err(|e| e.to_string())?;
        if !regular(&fd, true) || !regular(&path, true) || !id.matches(&fd) || !id.matches(&path) {
            return Err("移行先台帳の本文が差し替え・リンク化されています".into());
        }
        Ok(file)
    }
    fn verify_installed(&self, p: &Placement<'_>, saved: &Saved) -> Result<(), String> {
        let file = self.registry_file(p, saved)?;
        if saved.state != "installed" {
            return Err("移行先の端末内台帳が未完了です".into());
        }
        let expected = saved.image.as_ref().ok_or("台帳のコピー証拠がありません")?;
        if crate::storage::backup::relocation::control::registry_digest(
            &p.pending.target.join(REGISTRY),
        )? != *expected
        {
            return Err("配置済みの端末内台帳が改変されています。上書きしません".into());
        }
        verify_registry_image(
            &p.pending.target,
            &p.pending.target.join(REGISTRY),
            &p.pending.operation_id,
            &p.pending.manifest_hash,
            &saved.control.as_ref().ok_or("制御記録がありません")?.1,
        )?;
        drop(file);
        self.guard(p)
    }
}

pub(super) fn allowed(p: &Placement<'_>) -> Result<Vec<&'static str>, String> {
    let Some(local) = Local::open(p, false)? else {
        return Ok(vec![]);
    };
    let saved = local.saved()?;
    let mut allowed = vec![JOURNAL];
    if saved.registry.is_some() {
        local.registry_file(p, &saved)?;
        if saved.state == "installed" {
            local.verify_installed(p, &saved)?;
        }
        allowed.push(REGISTRY);
    }
    Ok(allowed)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryReceipt {
    pub operation_id: String,
    pub state: String,
    pub control_hash: String,
}
impl Placement<'_> {
    pub(crate) fn install_history(
        &self,
        handoff: &ControlHandoff<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<HistoryReceipt, String> {
        check(cancelled)?;
        let snapshot = handoff.snapshot();
        handoff.verify(cancelled)?;
        self.require_pending()?;
        self.verify(snapshot, cancelled)?;
        let mut local = Local::open(self, true)?.ok_or("移行先台帳を作成できません")?;
        let mut saved = local.saved()?;
        if saved.state == "installed" {
            local.verify_installed(self, &saved)?;
            handoff.check_compatible(
                saved
                    .control
                    .as_ref()
                    .ok_or("元の制御記録がありません")?
                    .0
                    .as_bytes(),
            )?;
            self.verify_native(snapshot, cancelled, &[JOURNAL, REGISTRY])?;
            return Ok(HistoryReceipt {
                operation_id: self.pending.operation_id.clone(),
                state: "installed".into(),
                control_hash: saved.control.unwrap().1,
            });
        }
        if saved.state == "pending" {
            let name = cstring(REGISTRY)?;
            let file = descriptor(unsafe {
                libc::openat(
                    self.root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            })?;
            crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
            crate::native_fs::sync_handle(&self.root).map_err(|e| e.to_string())?;
            let identity = serde_json::to_string(&Identity::read(
                &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
            )?)
            .map_err(|e| e.to_string())?;
            local.guard(self)?;
            local.db.execute("UPDATE handoff SET registry_identity=?1,registry_hash=?2,state='owned' WHERE id=1 AND state='pending'",params![identity,hash(identity.as_bytes())]).map_err(|e|e.to_string())?;
            local.committed(self)?;
            saved = local.saved()?;
        }
        let mut output = local.registry_file(self, &saved)?;
        check(cancelled)?;
        handoff.verify(cancelled)?;
        let image = handoff.build_registry_image(
            &self.pending.operation_id,
            &self.pending.target,
            self,
            cancelled,
        )?;
        let control = std::str::from_utf8(handoff.bytes()).map_err(|e| e.to_string())?;
        local.guard(self)?;
        local.db.execute("UPDATE handoff SET control_json=?1,control_hash=?2,image_size=?3,image_hash=?4 WHERE id=1 AND state='owned'",params![control,handoff.hash(),i64::try_from(image.size).map_err(|_|"台帳サイズが範囲外です")?,image.hash]).map_err(|e|e.to_string())?;
        local.committed(self)?;
        // Identity and the complete new plan are durable before any truncation.
        image.write_owned(&mut output, cancelled)?;
        crate::native_fs::sync_handle(&self.root).map_err(|e| e.to_string())?;
        local.registry_file(self, &local.saved()?)?;
        verify_registry_image(
            &self.pending.target,
            &self.pending.target.join(REGISTRY),
            &self.pending.operation_id,
            &self.pending.manifest_hash,
            handoff.hash(),
        )?;
        if crate::storage::backup::relocation::control::registry_digest(
            &self.pending.target.join(REGISTRY),
        )? != (image.size, image.hash)
        {
            return Err("配置した端末内台帳のSHAが一致しません".into());
        }
        handoff.verify(cancelled)?;
        self.verify_native(snapshot, cancelled, &[JOURNAL, REGISTRY])?;
        local.guard(self)?;
        local
            .db
            .execute(
                "UPDATE handoff SET state='installed' WHERE id=1 AND state='owned'",
                [],
            )
            .map_err(|e| e.to_string())?;
        local.committed(self)?;
        local.verify_installed(self, &local.saved()?)?;
        Ok(HistoryReceipt {
            operation_id: self.pending.operation_id.clone(),
            state: "installed".into(),
            control_hash: handoff.hash().into(),
        })
    }
}

// Read actual owned installed evidence, never a deserialized success receipt.
impl Placement<'_> {
    pub(crate) fn inspect_deletion_handoff(
        &self,
        handoff: &ControlHandoff<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::storage::deletion::relocation::receiver::Report, String> {
        handoff.verify(cancelled)?;
        self.verify(handoff.snapshot(), cancelled)?;
        let (control, hash, _) = installed_control(self)?;
        let report = crate::storage::deletion::relocation::receiver::inspect(
            handoff, self, &control, &hash, cancelled,
        )?;
        let (after, after_hash, _) = installed_control(self)?;
        if control != after || hash != after_hash {
            return Err("削除履歴の検査中に配置済み制御記録が変更されています".into());
        }
        Ok(report)
    }
}

pub(super) fn installed_control(p: &Placement<'_>) -> Result<(String, String, Identity), String> {
    let local = Local::open(p, false)?.ok_or("移行先の履歴台帳が欠落しています")?;
    let saved = local.saved()?;
    local.verify_installed(p, &saved)?;
    let (json, hash) = saved.control.ok_or("履歴の制御記録がありません")?;
    Ok((
        json,
        hash,
        saved.registry.ok_or("履歴台帳の所有証拠がありません")?,
    ))
}

// After admission, registry contents can change during normal recovery/use.
// Still require the original installed journal and the admitted registry inode.
pub(super) fn admitted_registry(
    p: &Placement<'_>,
    control: &str,
    registry: &Identity,
) -> Result<(), String> {
    let local = Local::open(p, false)?.ok_or("承認済みの履歴台帳が欠落しています")?;
    let saved = local.saved()?;
    if saved.state != "installed"
        || saved.control.as_ref().is_none_or(|c| c.1 != control)
        || saved.registry.as_ref() != Some(registry)
    {
        return Err("承認済みの履歴・台帳所有証拠が一致しません".into());
    }
    local.registry_file(p, &saved)?;
    local.guard(p)
}

pub(super) fn admitted_control(p: &Placement<'_>, expected: &str) -> Result<String, String> {
    let local = Local::open(p, false)?.ok_or("承認済みの履歴台帳がありません")?;
    let saved = local.saved()?;
    if saved.state != "installed" || saved.control.as_ref().is_none_or(|c| c.1 != expected) {
        return Err("承認済みの制御記録が一致しません".into());
    }
    local.registry_file(p, &saved)?;
    local.guard(p)?;
    Ok(saved.control.ok_or("制御記録がありません")?.0)
}

#[cfg(all(test, unix))]
mod tests;
