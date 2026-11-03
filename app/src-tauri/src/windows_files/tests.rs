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
    assert_eq!(
        security_metadata(&open_metadata(&path).unwrap()).unwrap(),
        (current_owner().unwrap(), true)
    );
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

#[test]
fn bundled_msix_hardlinks_are_limited_to_the_current_package_and_pin_parents() {
    use std::io::Read;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let package = root.join("package");
    std::fs::create_dir_all(package.join("runtime")).unwrap();
    let asset = package.join("runtime").join("catalog.json");
    std::fs::write(&asset, b"bundled catalog").unwrap();
    std::fs::hard_link(&asset, root.join("shared-by-another-package")).unwrap();
    assert!(read(&asset, false).is_err());
    let (mut file, pins) = read_bundled_in_package(&asset, Some(&package)).unwrap();
    assert_eq!(identity(&file).unwrap().2, 2);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"bundled catalog");
    assert_eq!(pins.len(), 2);
    assert!(OpenOptions::new().write(true).open(&asset).is_err());
    assert!(std::fs::rename(package.join("runtime"), package.join("moved")).is_err());
    assert!(read_bundled_in_package(&asset, None).is_err());
    assert!(
        crate::model_inspection::Source::open(&package.join("runtime"), "catalog.json").is_err()
    );
    // No public entry point accepts the test's simulated package root.
    if current_package_path().unwrap().is_none() {
        assert!(read_bundled(&asset).is_err());
    }
}

#[test]
fn bundled_msix_reads_reject_outside_sibling_and_traversal_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let package = root.join("package");
    std::fs::create_dir(&package).unwrap();
    let single = package.join("single.json");
    std::fs::write(&single, b"single").unwrap();
    assert!(read_bundled_in_package(&single, Some(&package)).is_ok());
    assert!(read_bundled_in_package(&single, None).is_ok());
    let sibling = root.join("package-other");
    std::fs::create_dir(&sibling).unwrap();
    let outside = sibling.join("outside.json");
    std::fs::write(&outside, b"outside").unwrap();
    std::fs::hard_link(&outside, package.join("shared.json")).unwrap();
    assert!(read_bundled_in_package(&outside, Some(&package)).is_err());
    assert!(read_bundled_in_package(
        &package
            .join("..")
            .join("package-other")
            .join("outside.json"),
        Some(&package)
    )
    .is_err());
    assert!(read_bundled_in_package(Path::new("single.json"), Some(&package)).is_err());
    assert!(read_bundled_in_package(&package, Some(&package)).is_err());
}

#[test]
fn bundled_msix_reads_reject_reparse_files_and_parents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let package = root.join("package");
    std::fs::create_dir_all(package.join("real")).unwrap();
    let asset = package.join("real").join("catalog.json");
    std::fs::write(&asset, b"catalog").unwrap();
    let leaf_link = package.join("linked.json");
    if let Err(error) = std::os::windows::fs::symlink_file(&asset, &leaf_link) {
        if error.raw_os_error() == Some(1314) {
            eprintln!("Windows developer mode/symlink privilege is unavailable");
            return;
        }
        panic!("could not create test symlink: {error}");
    }
    assert!(read_bundled_in_package(&leaf_link, Some(&package)).is_err());
    let parent_link = package.join("linked-parent");
    std::os::windows::fs::symlink_dir(package.join("real"), &parent_link).unwrap();
    assert!(read_bundled_in_package(&parent_link.join("catalog.json"), Some(&package)).is_err());
    let root_link = root.join("linked-package");
    std::os::windows::fs::symlink_dir(&package, &root_link).unwrap();
    assert!(read_bundled_in_package(
        &root_link.join("real").join("catalog.json"),
        Some(&root_link)
    )
    .is_err());
}
