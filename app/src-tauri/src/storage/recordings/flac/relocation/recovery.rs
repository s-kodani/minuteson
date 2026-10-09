//! A native-copy input correspondence is not consent. It only allows the same
//! immutable conversion plan after a fresh physical Review and explicit yes.
use super::*;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    hash: String,
    observed: Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Facts {
    input_hash: String,
    content_hash: String,
    pub(super) document: Option<Document>,
}
impl Facts {
    pub(super) fn validate(&self) -> Result<(), String> {
        if !inventory::valid_hash(&self.input_hash) || !inventory::valid_hash(&self.content_hash) {
            return Err("部分救出の移動元入力証拠が不正です".into());
        }
        if let Some(document) = &self.document {
            if !inventory::valid_hash(&document.hash) {
                return Err("部分救出の移動元記録のSHAが不正です".into());
            }
            document.observed.validate(false)?;
        }
        Ok(())
    }
}

// No Deserialize and private fields: only admitted aliases plus current input
// inspection can mint this. It cannot authorize a guessed output or old consent.
pub(in crate::storage::recordings) struct RecoveryRebind {
    plan_hash: String,
    facts: Facts,
    document: Option<Identity>,
}
impl RecoveryRebind {
    pub(in crate::storage::recordings) fn verify_document_before_conversion(
        &self,
        directory: &Directory,
        value: &Value,
    ) -> Result<(), String> {
        match directory.read_json("partial-recovery.json")? {
            Some((bytes, stamp)) => {
                if bytes != serde_json::to_vec(value).map_err(|e| e.to_string())? {
                    self.require_document(directory, &bytes, value)?;
                }
                stamp.require(&directory.required("partial-recovery.json", false)?)?;
                Ok(())
            }
            None if self.facts.document.is_none() => Ok(()),
            None => Err("部分救出の配置した記録が失われています".into()),
        }
    }
    pub(in crate::storage::recordings) fn input_hash(&self) -> &str {
        &self.facts.input_hash
    }
    pub(in crate::storage::recordings) fn verify_plan(&self, bytes: &[u8]) -> Result<(), String> {
        if hash(bytes) != self.plan_hash {
            return Err("部分救出の承認した元の変換計画が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::recordings) fn require_document(
        &self,
        directory: &Directory,
        bytes: &[u8],
        value: &Value,
    ) -> Result<(), String> {
        let original = self
            .facts
            .document
            .as_ref()
            .ok_or("部分救出の移動元記録がありません")?;
        let identity = self
            .document
            .as_ref()
            .ok_or("部分救出の配置した記録がありません")?;
        identity.require(&directory.required("partial-recovery.json", false)?)?;
        if hash(bytes) != original.hash || !same_document(bytes, value)? {
            return Err("部分救出の移動元記録と新しい確認範囲が一致しません".into());
        }
        Ok(())
    }
}

fn same_document(bytes: &[u8], current: &Value) -> Result<bool, String> {
    let mut original: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let review = original
        .pointer_mut("/partialRecovery/reviewHash")
        .ok_or("部分救出の移動元確認がありません")?;
    if !review.as_str().is_some_and(inventory::valid_hash) {
        return Err("部分救出の移動元確認SHAが不正です".into());
    }
    *review = current
        .pointer("/partialRecovery/reviewHash")
        .ok_or("部分救出の新しい確認がありません")?
        .clone();
    Ok(original == *current)
}

pub(in crate::storage::recordings) fn recovery_input(
    storage: &Storage,
    base: &Path,
    content_hash: &str,
) -> Result<Option<RecoveryRebind>, String> {
    let Some(aliases) = &storage.flac_history else {
        return Ok(None);
    };
    aliases.verify(storage)?;
    let base = base.to_str().ok_or("部分救出の所属が不正です")?;
    let Some((source, bound)) = aliases.plans.get(&(base.into(), PARTIAL_PLAN.into())) else {
        return Ok(None);
    };
    let Some(facts) = &source.recovery else {
        return Ok(None);
    };
    if facts.content_hash != content_hash {
        return Err("部分救出の原音声・録音・会議状態が移動元から変更されています".into());
    }
    let root = Directory::open(&storage.root.join(base))?;
    if root.identity != bound.recording {
        return Err("部分救出の配置した録音管理先が変更されています".into());
    }
    let (bytes, stamp) = root
        .read_json(PARTIAL_PLAN)?
        .ok_or("部分救出の元の計画がありません")?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if hash(&bytes) != source.plan_hash || plan.input_hash != facts.input_hash {
        return Err("部分救出の元の計画と承認した入力証拠が一致しません".into());
    }
    stamp.require(&root.required(PARTIAL_PLAN, false)?)?;
    aliases.verify(storage)?;
    Ok(Some(RecoveryRebind {
        plan_hash: source.plan_hash.clone(),
        facts: facts.clone(),
        document: bound.recovery_document.clone(),
    }))
}

pub(super) fn capture(
    storage: &Storage,
    snapshot: &NativeSnapshot<'_>,
    base: &str,
    plan: &Plan,
    plan_bytes: &[u8],
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Facts>, String> {
    // A normal plan is deliberately never a partial-recovery input proof.
    let root = Directory::open(&storage.root.join(base))?;
    let Some((partial_bytes, _)) = root.read_json(PARTIAL_PLAN)? else {
        return Ok(None);
    };
    if partial_bytes != plan_bytes {
        return Ok(None);
    }
    let session = base
        .rsplit('/')
        .next()
        .ok_or("部分救出の録音IDがありません")?;
    let review = match storage.inspect_partial_for_relocation(snapshot, session, cancelled) {
        Ok(review) => review,
        Err(_) => {
            partial::check_cancel(cancelled)?;
            return Ok(None);
        }
    };
    let content_hash = review.content_hash()?;
    let inherited = if review.input_hash()? != plan.input_hash {
        let Some(proof) = recovery_input(storage, Path::new(base), &content_hash)? else {
            return Ok(None);
        };
        if proof.input_hash() != plan.input_hash {
            return Ok(None);
        }
        true
    } else {
        false
    };
    let document = if let Some((bytes, stamp)) = root.read_json("partial-recovery.json")? {
        let expected = review.document();
        if !same_document(&bytes, &expected)?
            || (!inherited && bytes != serde_json::to_vec(&expected).map_err(|e| e.to_string())?)
        {
            return Ok(None);
        }
        let path = format!("{base}/partial-recovery.json");
        let observed = Identity::file(&root.required("partial-recovery.json", false)?)?;
        if source_identity(snapshot, &path)? != observed
            || snapshot
                .manifest()
                .entries
                .iter()
                .find(|e| e.relative_path == path)
                .and_then(|e| e.hash.as_deref())
                != Some(hash(&bytes).as_str())
        {
            return Err("部分救出の元の記録と固定した移動像が一致しません".into());
        }
        stamp.require(&root.required("partial-recovery.json", false)?)?;
        Some(Document {
            hash: hash(&bytes),
            observed,
        })
    } else {
        None
    };
    root.verify()?;
    let facts = Facts {
        input_hash: plan.input_hash.clone(),
        content_hash,
        document,
    };
    facts.validate()?;
    Ok(Some(facts))
}

pub(super) fn admit(
    admission: &NativeHistoryAdmission<'_, '_>,
    source: &Workspace,
    bound: &BoundWorkspace,
) -> Result<(), String> {
    let document = source.recovery.as_ref().and_then(|p| p.document.as_ref());
    match (document, &bound.recovery_document) {
        (None, None) => Ok(()),
        (Some(document), Some(identity)) => {
            identity.validate(false)?;
            let path = format!("{}/partial-recovery.json", source.base);
            if identity.registered()? != admission.owned_path(&path, false)?
                || admission.copied_file_hash(&path)? != document.hash
            {
                return Err("部分救出の記録は配置した実体・SHAではありません".into());
            }
            Ok(())
        }
        _ => Err("部分救出の移動元記録と配置した記録が一致しません".into()),
    }
}
