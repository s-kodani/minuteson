//! Move native-owned staging, then verify physical bytes once at destination.
//! Journals contain expectations and ownership, never fabricated inspection.
use super::*;
use crate::model_placement::{DirectoryIdentity, PlacementReceipt, PlacementRecord};

const TRANSFER: &str = "placement-intent.json";
const COPY: &str = "placement-copy.json";
const COMMITTED: &str = "placement-committed.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Transfer {
    version: u32,
    download_id: String,
    package_id: String,
    recipe_hash: String,
    review_hash: String,
    source: DirectoryIdentity,
    destination_root: DirectoryIdentity,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Copy {
    name: String,
    identity: DirectoryIdentity,
    transfer: Transfer,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Reference {
    package_id: String,
    state: &'static str,
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn optional<T: serde::de::DeserializeOwned>(
    directory: &Directory,
    name: &str,
) -> Result<Option<T>, String> {
    match std::fs::symlink_metadata(directory.path().join(name)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(_) => {
            let (value, source) = read_record(directory.path(), name, 4096)?;
            source.verify()?;
            Ok(Some(value))
        }
    }
}
fn save(directory: &Directory, name: &str, value: &impl Serialize) -> Result<(), String> {
    durable_file(
        directory,
        name,
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    )
}
impl Transfer {
    fn validate(&self, operation: &str, recipe: &str, review: &str) -> Result<(), String> {
        if self.version != 1
            || !uuid(&self.package_id)
            || self.download_id != operation
            || self.recipe_hash != recipe
            || self.review_hash != review
        {
            return Err("モデル移動の所有記録と取得・確認内容が一致しません".into());
        }
        Ok(())
    }
}
pub(super) fn reference(
    directory: &Directory,
    operation: &str,
    recipe: &str,
    review: &str,
) -> Result<Option<Reference>, String> {
    let Some(transfer) = optional::<Transfer>(directory, TRANSFER)? else {
        return Ok(None);
    };
    transfer.validate(operation, recipe, review)?;
    let committed = optional::<Transfer>(directory, COMMITTED)?;
    if committed.as_ref().is_some_and(|value| value != &transfer) {
        return Err("モデル移動の完了記録が一致しません".into());
    }
    Ok(Some(Reference {
        package_id: transfer.package_id,
        state: if committed.is_some() {
            "committed"
        } else {
            "pending"
        },
    }))
}
fn existing(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
fn validate_tree(
    directory: &Directory,
    recipe: &Recipe,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    use std::collections::BTreeSet;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let mut files: BTreeSet<String> = recipe
        .manifest
        .files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    files.insert(model_inspection::MANIFEST.into());
    let mut directories = BTreeSet::new();
    for name in &files {
        let mut prefix = name.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            directories.insert(parent.to_owned());
            prefix = parent;
        }
    }
    let mut pending = vec![String::new()];
    while let Some(relative) = pending.pop() {
        check(cancelled)?;
        let parent_path = crate::native_fs::join(directory.path(), &relative);
        let parent = Directory::open_existing(&parent_path)?;
        for entry in std::fs::read_dir(parent.path()).map_err(|e| e.to_string())? {
            check(cancelled)?;
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "モデルのファイル名が不正です")?;
            let name = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() && directories.remove(&name) {
                pending.push(name);
            } else if kind.is_file() && files.remove(&name) {
                let source = Source::open(directory.path(), &name)?;
                let metadata =
                    crate::native_fs::metadata(&source.file).map_err(|e| e.to_string())?;
                if metadata.nlink() != 1
                    || metadata.uid() != crate::native_fs::current_owner()?
                    || metadata.mode() & 0o077 != 0
                {
                    return Err("移動するモデルファイルの所有・共有・権限が不正です".into());
                }
            } else {
                return Err("モデル領域に配布定義外のファイルまたはリンクがあります".into());
            }
        }
        parent.verify()?;
    }
    if !files.is_empty() || !directories.is_empty() {
        return Err("モデルのファイル・フォルダーが欠損しています".into());
    }
    directory.verify()
}

/// Copy fallback creates a fresh owned attempt. Interrupted copies are retained;
/// no unknown/partial file is truncated or adopted by a later attempt.
fn copy_payload(
    source: &Path,
    destination: &Directory,
    recipe: &Recipe,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    for (name, expected) in std::iter::once((model_inspection::MANIFEST, None)).chain(
        recipe
            .manifest
            .files
            .iter()
            .map(|f| (f.relative_path.as_str(), f.expected_size)),
    ) {
        check(cancelled)?;
        let mut input = Source::open(source, name)?;
        let size = input.file.metadata().map_err(|e| e.to_string())?.len();
        if expected.is_some_and(|n| n != size) || (expected.is_none() && size > 65536) {
            return Err("コピー元のモデル容量が一致しません".into());
        }
        let (mut output, parent) = destination.create(name)?;
        let mut copied = 0u64;
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            check(cancelled)?;
            let n = input.file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > size {
                return Err("コピー中にモデル容量が変更されました".into());
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
        if copied != size {
            return Err("コピー中にモデルが短くなりました".into());
        }
        input.verify()?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        parent.sync()?;
    }
    destination.sync()
}
fn copy_and_move(
    operation: &Directory,
    root: &Directory,
    recipe: &Recipe,
    transfer: &Transfer,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    if root.available()?
        < recipe
            .review()?
            .total_bytes
            .saturating_add(16 * 1024 * 1024)
    {
        return Err("モデル管理領域の空き容量が不足しています".into());
    }
    let name = uuid::Uuid::new_v4().to_string();
    let copy = root.child(&name, true)?;
    let identity = copy.identity()?;
    copy_payload(&operation.path().join("payload"), &copy, recipe, cancelled)?;
    drop(copy); // Windows rename must not retain a non-delete-sharing pin.
    save(
        operation,
        COPY,
        &Copy {
            name: name.clone(),
            identity: identity.clone(),
            transfer: transfer.clone(),
        },
    )?;
    if !root.move_child(&name, root, &transfer.package_id, &identity)? {
        return Err("モデルコピー先のドライブが変更されています".into());
    }
    Ok(())
}

pub(super) fn place(
    storage: &crate::storage::Storage,
    models_root: &Path,
    staging_root: &Path,
    operation_id: &str,
    expected_recipe_hash: &str,
    package_id: &str,
    expected_review_hash: &str,
    cancelled: &impl Fn() -> bool,
    verifying: &impl Fn(),
) -> Result<PlacementReceipt, String> {
    check(cancelled)?;
    if !uuid(operation_id) || !uuid(package_id) {
        return Err("モデル移動の操作IDが不正です".into());
    }
    let staging = Directory::open_existing(staging_root)?;
    let operation = Directory::open_existing(&staging_root.join(operation_id))?;
    let (intent, intent_proof): (Intent, _) =
        read_record(operation.path(), INTENT, MAX_RECIPE + 1024)?;
    let review = intent.recipe.review()?;
    let (complete, complete_proof): (Completion, _) =
        read_record(operation.path(), COMPLETE, 1024)?;
    if intent.schema_version != 1
        || intent.operation_id != operation_id
        || intent.recipe_hash != expected_recipe_hash
        || review.recipe_hash != expected_recipe_hash
        || complete.schema_version != 1
        || complete.operation_id != operation_id
        || complete.recipe_hash != expected_recipe_hash
        || complete.review_hash != expected_review_hash
        || complete.total_bytes != review.total_bytes
    {
        return Err("モデル取得の意図・完了記録と移動要求が一致しません".into());
    }
    // Creating our transfer journal changes the operation directory timestamp.
    // Release those parent snapshots, then compare the records again at commit.
    intent_proof.verify()?;
    complete_proof.verify()?;
    drop(intent_proof);
    drop(complete_proof);
    let root = Directory::root(models_root)?;
    let transfer = if let Some(value) = optional::<Transfer>(&operation, TRANSFER)? {
        value.validate(operation_id, expected_recipe_hash, expected_review_hash)?;
        if value.package_id != package_id || value.destination_root != root.identity()? {
            return Err("再開するモデル配置先が元の所有記録と一致しません".into());
        }
        value
    } else {
        if existing(&root.path().join(package_id))? {
            return Err("モデル配置先に既存物があります。上書きしません".into());
        }
        let source = Directory::open_existing(&operation.path().join("payload"))?.identity()?;
        let value = Transfer {
            version: 1,
            download_id: operation_id.into(),
            package_id: package_id.into(),
            recipe_hash: expected_recipe_hash.into(),
            review_hash: expected_review_hash.into(),
            source,
            destination_root: root.identity()?,
        };
        save(&operation, TRANSFER, &value)?;
        value
    };
    let copied = optional::<Copy>(&operation, COPY)?;
    if optional::<Transfer>(&operation, COMMITTED)?
        .as_ref()
        .is_some_and(|value| value != &transfer)
    {
        return Err("モデル移動の完了記録が一致しません".into());
    }
    if let Some(copy) = &copied {
        if copy.transfer != transfer || !uuid(&copy.name) {
            return Err("モデルコピーの所有記録が一致しません".into());
        }
    }
    check(cancelled)?;
    if !existing(&root.path().join(package_id))? {
        if let Some(copy) = &copied {
            if !root.move_child(&copy.name, &root, package_id, &copy.identity)? {
                return Err("モデルコピー先のドライブが変更されています".into());
            }
        } else if !operation.move_child("payload", &root, package_id, &transfer.source)? {
            // Cross-volume only. Keep original staging until final verification.
            copy_and_move(&operation, &root, &intent.recipe, &transfer, cancelled)?;
        }
    }
    let destination = Directory::open_existing(&root.path().join(package_id))?;
    let copy = optional::<Copy>(&operation, COPY)?;
    let expected_identity = copy.as_ref().map_or(&transfer.source, |c| &c.identity);
    if &destination.identity()? != expected_identity {
        return Err("モデル配置先の実体が移動記録と一致しません".into());
    }
    check(cancelled)?;
    verifying();
    validate_tree(&destination, &intent.recipe, cancelled)?;
    // This is the single full-byte inspection in the placement stage.
    let report = model_inspection::inspect(destination.path(), cancelled)?;
    let manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&intent.recipe.manifest).map_err(|e| e.to_string())?)
    );
    if report.manifest != intent.recipe.manifest
        || report.manifest_hash != manifest_hash
        || report.review_hash != expected_review_hash
        || report.total_bytes != review.total_bytes
    {
        return Err("配置したモデルが配布定義・取得完了記録と一致しません".into());
    }
    #[cfg(unix)]
    for file in &report.files {
        use std::os::unix::fs::PermissionsExt;
        let source = Source::open(destination.path(), &file.relative_path)?;
        source
            .file
            .set_permissions(std::fs::Permissions::from_mode(
                if file.role == model_inspection::Role::Executable {
                    0o700
                } else {
                    0o600
                },
            ))
            .map_err(|e| e.to_string())?;
        source.file.sync_all().map_err(|e| e.to_string())?;
    }
    destination.sync()?;
    let (latest_intent, _): (Intent, _) = read_record(operation.path(), INTENT, MAX_RECIPE + 1024)?;
    let (latest_complete, _): (Completion, _) = read_record(operation.path(), COMPLETE, 1024)?;
    if serde_json::to_vec(&latest_intent).map_err(|e| e.to_string())?
        != serde_json::to_vec(&intent).map_err(|e| e.to_string())?
        || serde_json::to_vec(&latest_complete).map_err(|e| e.to_string())?
            != serde_json::to_vec(&complete).map_err(|e| e.to_string())?
    {
        return Err("モデル取得記録が配置中に変更されました".into());
    }
    staging.verify()?;
    operation.verify()?;
    root.verify()?;
    let record =
        PlacementRecord::downloaded(package_id, &report, intent.recipe.origin(operation_id)?)?;
    let saved = storage.model_placement_receipt(&record)?;
    check(cancelled)?;
    if saved.is_none() {
        storage.begin_model_placement(&record)?;
    }
    if !saved.as_ref().is_some_and(|r| r.placement_recorded) {
        storage.finish_model_placement(&record)?;
    }
    if optional::<Transfer>(&operation, COMMITTED)?.is_none() {
        save(&operation, COMMITTED, &transfer)?;
    }
    storage
        .model_placement_receipt(&record)?
        .ok_or("モデル配置の完了記録がありません".into())
}

#[cfg(test)]
mod tests;
