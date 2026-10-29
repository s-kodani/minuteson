use super::*;
#[test]
fn windows_model_placement_records_actual_namespace_guarantee_and_private_acl() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("private-models");
    let directory = crate::model_placement::fs::Directory::root(&root).unwrap();
    assert!(security_metadata(&directory_for_test(&root)).unwrap().1);
    let (file, parent) = directory.create("nested/receipt.json").unwrap();
    use std::io::Write;
    (&file).write_all(b"flushed payload").unwrap();
    file.sync_all().unwrap();
    assert!(security_metadata(&file).unwrap().1);
    assert!(matches!(
        parent.sync_namespace().unwrap(),
        mon_native::durability::NamespaceDurability::Flushed
            | mon_native::durability::NamespaceDurability::RestartRecoverable
    ));
}
fn directory_for_test(path: &std::path::Path) -> std::fs::File {
    directory(path).unwrap()
}
#[test]
fn private_root_reopen_keeps_descendant_change_proofs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    set_private(&root).unwrap();
    let path = root.join("retained-proof.json");
    std::fs::write(&path, b"retained").unwrap();
    let before = crate::audio::reuse::stamp(&crate::native_fs::symlink_metadata(&path).unwrap());
    set_private(&root).unwrap();
    set_private(&root).unwrap();
    assert_eq!(
        crate::audio::reuse::stamp(&crate::native_fs::symlink_metadata(&path).unwrap()),
        before
    );
}
#[test]
fn windows_parity_private_files_retain_sqlite_writes_and_reject_public_acl() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("owned.sqlite3");
    std::fs::write(&path, b"").unwrap();
    set_private(&path).unwrap();
    assert!(security_metadata(&open_metadata(&path).unwrap()).unwrap().1);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE v(value); INSERT INTO v VALUES(1);")
        .unwrap();
    let held = open_metadata(&path).unwrap();
    db.execute_batch("INSERT INTO v VALUES(2);").unwrap();
    assert_eq!(Identity::file(&held).unwrap(), Identity::at(&path).unwrap());
}
#[test]
fn windows_parity_exact_identity_removal_preserves_replacements_and_hardlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("owned");
    std::fs::write(&path, b"first").unwrap();
    let old = Identity::at(&path).unwrap();
    std::fs::rename(&path, root.join("retained")).unwrap();
    std::fs::write(&path, b"second").unwrap();
    assert!(remove_owned(&path, &old, false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    let next = Identity::at(&path).unwrap();
    std::fs::hard_link(&path, root.join("alias")).unwrap();
    assert!(remove_owned(&path, &next, false).is_err());
    std::fs::remove_file(root.join("alias")).unwrap();
    remove_owned(&path, &next, false).unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read(root.join("retained")).unwrap(), b"first");
}
