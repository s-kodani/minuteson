//! Display names identify the embedding weights, independently of installed
//! packages, segmentation variants, runtime builds, and the current profile.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Definition {
    speaker_embedding_name: Option<String>,
    files: Vec<File>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    role: String,
    expected_hash: String,
}
fn catalog() -> Result<BTreeMap<String, String>, String> {
    let definitions: Vec<Definition> =
        serde_json::from_str(include_str!("../../model-catalog/model-definitions.json"))
            .map_err(|e| e.to_string())?;
    let mut names = BTreeMap::new();
    for definition in definitions {
        for file in definition
            .files
            .into_iter()
            .filter(|f| f.role == "speaker_embedding")
        {
            let name = definition
                .speaker_embedding_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .ok_or("話者識別モデルの表示名がありません")?;
            if names
                .insert(file.expected_hash, name.to_owned())
                .is_some_and(|old| old != name)
            {
                return Err("同じ話者識別モデルの表示名が一致しません".into());
            }
        }
    }
    Ok(names)
}
pub(crate) fn name(hash: &str) -> Option<String> {
    static NAMES: OnceLock<Result<BTreeMap<String, String>, String>> = OnceLock::new();
    NAMES.get_or_init(catalog).as_ref().ok()?.get(hash).cloned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_embedding_names_survive_package_removal_and_segmentation_variants() {
        let names = super::catalog().unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(
            super::name("1a331345f04805badbb495c775a6ddffcdd1a732567d5ec8b3d5749e3c7a5e4b")
                .as_deref(),
            Some("3D-Speaker ERes2Net base")
        );
        assert!(super::name(&"f".repeat(64)).is_none());
    }
}
