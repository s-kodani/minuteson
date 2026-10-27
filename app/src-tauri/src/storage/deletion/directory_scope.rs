//! A whole-folder request binds one directory identity, including all contents.
//! Only a journaled reserved root can be recursively removed. Legacy byte-fixed
//! requests never enter this lane.
use super::fs::{Directory, Proof};

pub(super) enum Boundary {
    Armed(Proof),
    Staged(Proof),
}

fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除を停止しました。残りの削除は再開できます".into())
    } else {
        Ok(())
    }
}

pub(super) fn verify(
    parent: &Directory,
    original: &str,
    reserved: &str,
    expected: Option<&Proof>,
    pending: Option<(&Proof, Option<&Proof>)>,
    removed: bool,
) -> Result<(), String> {
    parent.verify()?;
    let source = parent.child(original)?;
    let staged = parent.child(reserved)?;
    if removed || expected.is_none() {
        return if source.is_none() && staged.is_none() {
            Ok(())
        } else {
            Err("削除済み・欠落したフォルダーが再出現しています".into())
        };
    }
    let expected = expected.unwrap();
    match (source, staged, pending) {
        (Some(source), None, None | Some((_, None))) => source.verify_identity(&expected.identity),
        (None, Some(staged), Some((before, _))) => staged.verify_identity(&before.identity),
        (None, None, Some((_, Some(_)))) => Ok(()), // crash after removal
        _ => Err("削除対象のフォルダーまたは予約先が変更されています".into()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn remove(
    parent: &Directory,
    original: &str,
    reserved: &str,
    expected: &Proof,
    pending: Option<(Proof, Option<Proof>)>,
    check: &impl Fn() -> bool,
    mut journal: impl FnMut(Boundary) -> Result<(), String>,
) -> Result<(), String> {
    cancelled(check)?;
    verify(
        parent,
        original,
        reserved,
        Some(expected),
        pending.as_ref().map(|(b, s)| (b, s.as_ref())),
        false,
    )?;
    let (before, staged) = match pending {
        Some(pending) => pending,
        None => {
            let proof = parent
                .child(original)?
                .ok_or("削除対象がありません")?
                .proof()?
                .directory_scope();
            journal(Boundary::Armed(proof.clone()))?;
            (proof, None)
        }
    };
    cancelled(check)?;
    if parent.child(original)?.is_some() {
        parent
            .child(original)?
            .ok_or("削除対象がありません")?
            .verify_identity(&before.identity)?;
        parent.rename_exclusive(original, reserved)?;
    }
    if let Some(directory) = parent.child(reserved)? {
        directory.verify_identity(&before.identity)?;
        if staged.is_none() {
            journal(Boundary::Staged(directory.proof()?.directory_scope()))?;
        }
        drop(directory);
        cancelled(check)?;
        // Checkpoint only folder boundaries; recursive unlink never reads file
        // contents or checkpoints every individual entry.
        parent.remove_reserved_tree(reserved, &before.identity, check)?;
    } else if staged.is_none() {
        return Err("移動対象が消失しています。削除結果を確認できません".into());
    }
    parent.sync()?;
    verify(parent, original, reserved, None, None, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn directory_scope_resumes_partial_recursive_removal_and_checks_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let source = path.join("target");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("nested/one"), b"one").unwrap();
        std::fs::write(source.join("nested/two"), b"two").unwrap();
        let parent = Directory::root(&path).unwrap();
        let expected = parent
            .child("target")
            .unwrap()
            .unwrap()
            .proof()
            .unwrap()
            .directory_scope();
        let pending = RefCell::new(None);
        let stop = Cell::new(false);
        let reserved = ".mon-delete-directory-fixture";
        assert!(remove(
            &parent,
            "target",
            reserved,
            &expected,
            None,
            &|| stop.get(),
            |boundary| {
                match boundary {
                    Boundary::Armed(before) => *pending.borrow_mut() = Some((before, None)),
                    Boundary::Staged(staged) => {
                        pending.borrow_mut().as_mut().unwrap().1 = Some(staged);
                        stop.set(true);
                    }
                }
                Ok(())
            }
        )
        .is_err());
        assert!(path.join(reserved).exists());
        assert!(!source.exists());
        // Simulate already removed children and changed directory timestamps.
        std::fs::remove_file(path.join(reserved).join("nested/one")).unwrap();
        drop(parent);
        let parent = Directory::root(&path).unwrap();
        remove(
            &parent,
            "target",
            reserved,
            &expected,
            pending.borrow().clone(),
            &|| false,
            |_| panic!("already staged"),
        )
        .unwrap();
        assert!(!path.join(reserved).exists());
        // A crash between unlink and saving the outer completion can replay.
        remove(
            &parent,
            "target",
            reserved,
            &expected,
            pending.borrow().clone(),
            &|| false,
            |_| panic!("already removed"),
        )
        .unwrap();
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("replacement"), b"keep").unwrap();
        assert!(remove(
            &parent,
            "target",
            reserved,
            &expected,
            pending.borrow().clone(),
            &|| false,
            |_| Ok(())
        )
        .is_err());
        assert!(source.join("replacement").exists());
    }

    #[test]
    fn directory_scope_armed_request_can_resume_before_or_after_rename() {
        for renamed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().canonicalize().unwrap();
            std::fs::create_dir(path.join("target")).unwrap();
            std::fs::write(path.join("target/file"), b"content").unwrap();
            let parent = Directory::root(&path).unwrap();
            let proof = parent
                .child("target")
                .unwrap()
                .unwrap()
                .proof()
                .unwrap()
                .directory_scope();
            let reserved = ".mon-delete-armed-fixture";
            if renamed {
                parent.rename_exclusive("target", reserved).unwrap();
            }
            let staged = Cell::new(false);
            remove(
                &parent,
                "target",
                reserved,
                &proof,
                Some((proof.clone(), None)),
                &|| false,
                |boundary| {
                    assert!(matches!(boundary, Boundary::Staged(_)));
                    staged.set(true);
                    Ok(())
                },
            )
            .unwrap();
            assert!(staged.get());
            assert!(!path.join("target").exists() && !path.join(reserved).exists());
        }
    }
}
