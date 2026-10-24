//! Confirmed recovery of unfinished data work. Keep is not recovered/published.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    Scratch,
    RestoreAddition,
}
impl Source {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Scratch => "scratch",
            Self::RestoreAddition => "restore_addition",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub source: Source,
    pub operation_id: String,
    pub paths: Vec<String>,
    pub registered_paths: usize,
    pub reason: String,
    pub retained: bool,
}
pub(super) struct Evidence {
    pub item: Item,
    pub state: String,
    pub intent_hash: String,
    pub status_hash: String,
    pub live: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub supported: bool,
    pub root: String,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub confirmation_id: String,
    pub item: Item,
    pub root: String,
    pub observed_paths: usize,
    pub inspection_complete: bool,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Recovered,
    Retained,
    Attention,
    Published,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub source: Source,
    pub operation_id: String,
    pub outcome: Outcome,
    pub reason: Option<String>,
}
pub(crate) struct Prepared {
    preview: Preview,
    root: PathBuf,
    intent: String,
    observation: String,
    terminal: Option<(bool, Receipt)>,
}

// Metadata inventories include unknown names and unreadable-directory markers.
// Do not read payloads or follow links. A keep decision explicitly preserves
// even contents we cannot inspect, rather than labelling them safely reclaimed.
fn observe(
    root: &Path,
    e: &Evidence,
    cancelled: &impl Fn() -> bool,
) -> Result<(String, usize, bool), String> {
    let mut digest = Sha256::new();
    digest.update(e.intent_hash.as_bytes());
    digest.update(e.status_hash.as_bytes());
    let mut count = 0usize;
    let mut complete = !e.item.paths.is_empty();
    let mut queue = Vec::new();
    for relative in &e.item.paths {
        let mut path = root.to_path_buf();
        let parts: Vec<_> = Path::new(relative).components().collect();
        let mut blocked = false;
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            path.push(part.as_os_str());
            match crate::native_fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() && path.canonicalize().is_ok_and(|p| p == path) => {}
                result => {
                    digest.update(format!("blocked:{path:?}:{result:?}"));
                    complete = false;
                    blocked = true;
                    break;
                }
            }
        }
        if !blocked {
            queue.push(root.join(relative));
        }
    }
    while let Some(path) = queue.pop() {
        check_cancelled(cancelled)?;
        count += 1;
        if count > 200_000 {
            return Err("対象の表示が安全上限を超えています。ファイルを保持します".into());
        }
        digest.update(format!(
            "path:{:?}",
            path.strip_prefix(root).map_err(|e| e.to_string())?
        ));
        let m = match crate::native_fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(error) => {
                digest.update(format!("missing:{:?}", error.kind()));
                if error.kind() != std::io::ErrorKind::NotFound {
                    complete = false;
                }
                continue;
            }
        };
        #[cfg(any(unix, windows))]
        {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            digest.update(format!(
                "{}:{}:{}:{}:{}:{}:{}:{}:{}",
                m.dev(),
                m.ino(),
                m.mode(),
                m.nlink(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec()
            ));
        }
        digest.update(format!("birth:{:?}:kind:{:?}", m.created(), m.file_type()));
        if m.is_dir() {
            if !path.canonicalize().is_ok_and(|p| p == path) {
                complete = false;
                continue;
            }
            let mut children = match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut children = Vec::new();
                    for entry in entries {
                        check_cancelled(cancelled)?;
                        if children
                            .len()
                            .saturating_add(queue.len())
                            .saturating_add(count)
                            >= 200_000
                        {
                            return Err(
                                "対象の表示が安全上限を超えています。ファイルを保持します".into()
                            );
                        }
                        children.push(entry.map_err(|e| e.to_string())?.path());
                    }
                    children
                }
                Err(error) => {
                    digest.update(format!("unreadable:{:?}", error.kind()));
                    complete = false;
                    continue;
                }
            };
            if children
                .len()
                .saturating_add(queue.len())
                .saturating_add(count)
                > 200_000
            {
                return Err("対象の表示が安全上限を超えています。ファイルを保持します".into());
            }
            children.sort();
            queue.extend(children);
        }
    }
    Ok((format!("{:x}", digest.finalize()), count, complete))
}

impl Storage {
    fn recovery_registry(&self) -> Result<&scratch::Registry, String> {
        self.scratch
            .as_deref()
            .ok_or_else(|| "Windowsの回収操作は保留中です".into())
    }
    fn recovery_evidence(&self, source: Source, id: &str) -> Result<Evidence, String> {
        let registry = self.recovery_registry()?;
        registry.guard()?;
        match source {
            Source::Scratch => registry.scratch_evidence(id),
            Source::RestoreAddition => {
                let db = self
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                publication::journal::recovery_evidence(&db, id)
            }
        }
    }
    pub(crate) fn recovery_listing(&self) -> Result<Listing, String> {
        let mut listing = Listing {
            supported: self.scratch.is_some(),
            root: self.root.to_string_lossy().into(),
            items: vec![],
        };
        let Some(registry) = &self.scratch else {
            return Ok(listing);
        };
        let mut pending = registry
            .pending_scratch()?
            .into_iter()
            .map(|id| (Source::Scratch, id))
            .collect::<Vec<_>>();
        let ids = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let mut q=db.prepare("SELECT id FROM maintenance_operations WHERE kind='restore_addition' AND state NOT IN ('completed','recovered') ORDER BY id LIMIT 2001").map_err(|e|e.to_string())?;
            let result = q
                .query_map([], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<String>, _>>()
                .map_err(|e| e.to_string())?;
            result
        };
        if pending.len().saturating_add(ids.len()) > 2000 {
            return Err("要確認記録が表示上限を超えています。既存ファイルは保持します".into());
        }
        pending.extend(ids.into_iter().map(|id| (Source::RestoreAddition, id)));
        for (source, id) in pending {
            let mut evidence = self.recovery_evidence(source, &id)?;
            if evidence.live {
                continue;
            }
            evidence.item.retained = registry.retained(source.key(), &id, &evidence.intent_hash)?;
            listing.items.push(evidence.item);
        }
        Ok(listing)
    }
    pub(crate) fn prepare_recovery(
        &self,
        source: Source,
        id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Prepared, String> {
        let _maintenance = self.backup_maintenance()?;
        let e = self.recovery_evidence(source, id)?;
        if e.live
            || matches!(e.state.as_str(), "recovered" | "completed")
            || self
                .recovery_registry()?
                .retained(source.key(), id, &e.intent_hash)?
        {
            return Err("この対象は利用中または確認済みです。一覧を再確認してください".into());
        }
        let (observation, observed_paths, inspection_complete) =
            observe(&self.root, &e, cancelled)?;
        Ok(Prepared {
            preview: Preview {
                confirmation_id: Uuid::new_v4().to_string(),
                item: e.item,
                root: self.root.to_string_lossy().into(),
                observed_paths,
                inspection_complete,
            },
            root: self.root.clone(),
            intent: e.intent_hash,
            observation,
            terminal: None,
        })
    }
}
impl Prepared {
    pub(crate) fn preview(&self) -> Preview {
        self.preview.clone()
    }
    pub(crate) fn resolve(
        &mut self,
        storage: &Storage,
        confirmation: &str,
        keep: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        if self.root != storage.root || self.preview.confirmation_id != confirmation {
            return Err("回収の確認対象が変更されています".into());
        }
        if let Some((decision, receipt)) = &self.terminal {
            if *decision != keep {
                return Err("確認済みの判断を別の操作へ変更できません".into());
            }
            return Ok(receipt.clone());
        }
        let maintenance = storage.backup_maintenance()?;
        let source = self.preview.item.source;
        let id = &self.preview.item.operation_id;
        let registry = storage.recovery_registry()?;
        let current = storage.recovery_evidence(source, id)?;
        if current.live
            || current.intent_hash != self.intent
            || observe(&storage.root, &current, cancelled)?.0 != self.observation
        {
            return Err(
                "確認後に対象・記録・状態が変更されています。最新の内容を再確認してください".into(),
            );
        }
        check_cancelled(cancelled)?;
        if keep {
            registry.keep(
                source.key(),
                id,
                &self.intent,
                &self.observation,
                confirmation,
            )?;
        } else {
            match source {
                Source::Scratch => {
                    let result = registry.recover_selected(id);
                    if let Err(error) = result {
                        registry.guard()?;
                        let after = registry.scratch_evidence(id)?;
                        if after.state != "attention" {
                            return Err(error);
                        }
                    }
                }
                Source::RestoreAddition => {
                    let db = storage
                        .connection
                        .lock()
                        .map_err(|_| "DBロックを取得できません")?;
                    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                        .map_err(|e| e.to_string())?;
                    let result =
                        publication::journal::recover_selected(&storage.root, &db, id, registry);
                    if let Err(error) =
                        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                            AuthAction::Read { .. }
                            | AuthAction::Select
                            | AuthAction::Function { .. }
                            | AuthAction::Recursive => Authorization::Allow,
                            _ => Authorization::Deny,
                        }))
                    {
                        maintenance.fail_closed();
                        return Err(error.to_string());
                    }
                    result?;
                }
            }
        }
        let after = storage.recovery_evidence(source, id)?;
        let outcome = if keep {
            Outcome::Retained
        } else {
            match after.state.as_str() {
                "recovered" => Outcome::Recovered,
                "completed" => Outcome::Published,
                _ => Outcome::Attention,
            }
        };
        let receipt = Receipt {
            source,
            operation_id: id.clone(),
            outcome,
            reason: (outcome == Outcome::Attention).then_some(after.item.reason),
        };
        self.terminal = Some((keep, receipt.clone()));
        Ok(receipt)
    }
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, unix))]
pub(crate) fn unresolved_for_controller_test(storage: &Storage) -> Item {
    let stage = storage.backup_stage().unwrap();
    std::fs::write(stage.path().join("unknown"), b"keep fixture").unwrap();
    drop(stage);
    storage.recovery_listing().unwrap().items.remove(0)
}
