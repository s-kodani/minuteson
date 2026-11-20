use super::*;
use crate::storage::deletion::fs::NativeProof as _;

fn directory_proof(journal: &Journal) -> &Proof {
    journal.source.observations[0].proof.as_ref().unwrap()
}

fn tree(
    directory: &Directory,
    path: &str,
    expected: &BTreeMap<&str, &Observation>,
    remove: bool,
    check: &impl Fn() -> bool,
) -> Result<usize, String> {
    super::super::cancel(check)?;
    let mut seen = 1;
    for name in directory.names(super::super::MAX_ENTRIES)? {
        super::super::cancel(check)?;
        let relative = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        let observation = expected
            .get(relative.as_str())
            .ok_or("整理対象に未登録の項目があります")?;
        let proof = observation
            .proof
            .as_ref()
            .ok_or("整理対象の実体がありません")?;
        if observation.entry.directory {
            let child = directory
                .child(&name)?
                .ok_or("整理対象のフォルダーが消失しました")?;
            child.verify_identity(&proof.identity)?;
            seen += tree(&child, &relative, expected, remove, check)?;
            if remove {
                let empty = child.proof()?;
                drop(child);
                super::super::cancel(check)?;
                directory.unlink_checked_child(&name, true, &empty)?;
            }
        } else {
            let file = directory.regular(&name)?;
            if Proof::read(&crate::native_fs::metadata(&file).map_err(|error| error.to_string())?)?
                != *proof
            {
                return Err("整理対象のファイルが変更されています".into());
            }
            if remove {
                super::super::cancel(check)?;
                directory.unlink_checked_child(&name, false, proof)?;
            }
            seen += 1;
        }
    }
    directory.verify()?;
    Ok(seen)
}

fn transition(
    storage: &Storage,
    journal: &mut Journal,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    update: impl FnOnce(&mut Journal),
) -> Result<(), String> {
    let mut next = journal.clone();
    update(&mut next);
    save(storage, &next, Some(journal), lease, check)?;
    *journal = next;
    Ok(())
}

pub(super) fn run(
    storage: &Storage,
    journal: &mut Journal,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    use crate::storage::deletion::directory_scope;

    active(storage, lease, check)?;
    {
        let db = crate::operation_observation::lock(&storage.connection)?;
        scope(storage, &db, journal, check)?;
    }
    let root = Path::new(&journal.models_root);
    let expected_root = directory_proof(journal).clone();
    let models = Directory::root(root)?;
    models.verify_identity(&journal.source.models_root)?;
    let original = journal.source.record.package_id.clone();
    let reserved = format!(".mon-delete-cleanup-{original}-folder");
    let pending = journal
        .pending
        .as_ref()
        .map(|value| (&value.before, value.staged.as_ref()));
    directory_scope::verify(
        &models,
        &original,
        &reserved,
        Some(&expected_root),
        pending,
        journal.complete(),
    )?;
    if journal.complete() {
        return Ok(());
    }

    // Windows may pin files during hashing. Release those handles before
    // moving the parent directory, then validate the same objects by identity.
    super::super::super::verification::release_pins();
    let before = expected_root.directory_scope();
    let staged = journal
        .pending
        .as_ref()
        .is_some_and(|pending| pending.staged.is_some());
    if let Some(directory) = models.child(&original)? {
        let expected = journal
            .source
            .observations
            .iter()
            .map(|item| (item.entry.relative_path.as_str(), item))
            .collect();
        super::super::verify_metadata_tree(&directory, "", &expected, check)?;
        drop(directory);
        active(storage, lease, check)?;
        models.rename_exclusive(&original, &reserved)?;
    }
    if let Some(directory) = models.child(&reserved)? {
        directory.verify_identity(&before.identity)?;
        {
            let expected = journal
                .source
                .observations
                .iter()
                .map(|item| (item.entry.relative_path.as_str(), item))
                .collect();
            let found = tree(&directory, "", &expected, false, check)?;
            if !staged && found != expected.len() {
                return Err("整理対象に欠落した項目があります".into());
            }
        }
        if !staged {
            transition(storage, journal, lease, check, |next| {
                next.pending.as_mut().unwrap().staged = Some(before.clone());
            })?;
        } else {
            let db = crate::operation_observation::lock(&storage.connection)?;
            scope(storage, &db, journal, check)?;
        }
        let expected = journal
            .source
            .observations
            .iter()
            .map(|item| (item.entry.relative_path.as_str(), item))
            .collect();
        tree(&directory, "", &expected, true, check)?;
        let empty = directory.proof()?;
        drop(directory);
        active(storage, lease, check)?;
        models.unlink_checked_child(&reserved, true, &empty)?;
    } else if !staged {
        return Err("予約済みのモデルフォルダーが消失しました".into());
    }
    models.sync()?;
    directory_scope::verify(&models, &original, &reserved, None, None, true)?;
    transition(storage, journal, lease, check, |next| {
        next.next = 1;
        next.pending = None;
    })
}
