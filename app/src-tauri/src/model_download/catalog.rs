//! Bounded, read-only metadata listing. A completion marker is not proof of
//! physical bytes: only explicit readback can grant a verified staging view.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Catalog {
    pub supported: bool,
    pub entries: Vec<Entry>,
    pub unknown_entries: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    operation_id: String,
    recipe_hash: Option<String>,
    model_name: Option<String>,
    model_version: Option<String>,
    total_bytes: Option<u64>,
    state: &'static str,
}
fn entry(root: &Path, id: String) -> Entry {
    let mut entry = Entry {
        operation_id: id.clone(),
        recipe_hash: None,
        model_name: None,
        model_version: None,
        total_bytes: None,
        state: "attention",
    };
    let result = (|| -> Result<(), String> {
        let operation = Directory::open_existing(&root.join(&id))?;
        let (intent, source): (Intent, _) =
            read_record(operation.path(), INTENT, MAX_RECIPE + 1024)?;
        let review = intent.recipe.review()?;
        if intent.schema_version != 1
            || intent.operation_id != id
            || intent.recipe_hash != review.recipe_hash
        {
            return Err("取得意図が一致しません".into());
        }
        entry.recipe_hash = Some(review.recipe_hash.clone());
        entry.model_name = Some(review.manifest.model_name);
        entry.model_version = Some(review.manifest.model_version);
        entry.total_bytes = Some(review.total_bytes);
        match std::fs::symlink_metadata(operation.path().join(COMPLETE)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => entry.state = "incomplete",
            Err(_) => return Err("完了記録を読めません".into()),
            Ok(_) => {
                let (completion, proof): (Completion, _) =
                    read_record(operation.path(), COMPLETE, 1024)?;
                if completion.schema_version != 1
                    || completion.operation_id != id
                    || completion.recipe_hash != review.recipe_hash
                    || completion.total_bytes != review.total_bytes
                    || !crate::model_download_ipc::hash(&completion.review_hash)
                {
                    return Err("完了記録が一致しません".into());
                }
                proof.verify()?;
                entry.state = "completion_recorded";
            }
        }
        source.verify()?;
        operation.verify_unchanged()?;
        Ok(())
    })();
    if result.is_err() {
        entry.state = "attention";
    }
    entry
}
pub(crate) fn catalog(root: &Path, cancelled: &impl Fn() -> bool) -> Result<Catalog, String> {
    check(cancelled)?;
    match std::fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Catalog {
                supported: true,
                entries: vec![],
                unknown_entries: 0,
            })
        }
        Err(_) => return Err("取得履歴の管理領域を読み取れません。既存物は保全します".into()),
        Ok(_) => {}
    }
    let directory = Directory::open_existing(root)?;
    let mut ids = Vec::new();
    let mut unknown = 0;
    let mut count = 0;
    for item in std::fs::read_dir(root).map_err(|_| "取得履歴の一覧を読めません")? {
        check(cancelled)?;
        count += 1;
        if count > 4096 {
            return Err("取得履歴の件数上限を超えています。省略・削除は行いません".into());
        }
        let item = item.map_err(|_| "取得履歴の一覧を読めません")?;
        match item.file_name().to_str() {
            Some(id) if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) => {
                ids.push(id.to_owned())
            }
            _ => unknown += 1,
        }
    }
    ids.sort();
    let mut entries = Vec::new();
    for id in ids {
        check(cancelled)?;
        entries.push(entry(root, id));
    }
    directory.verify_unchanged()?;
    Ok(Catalog {
        supported: true,
        entries,
        unknown_entries: unknown,
    })
}
