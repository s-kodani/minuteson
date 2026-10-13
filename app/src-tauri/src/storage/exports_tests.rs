use super::*;

#[test]
fn filenames_are_portable_bounded_and_do_not_need_structured_minutes() {
    for title in ["a/b:c*?\n<>|\\\"", "../", "", " . "] {
        let name = safe_filename(title, 0).unwrap();
        assert!(name.ends_with("_議事録.md"));
        assert!(!name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)));
    }
    assert!(safe_filename("定例", 0)
        .unwrap()
        .ends_with("_定例_議事録.md"));
    assert!(safe_filename(&"会".repeat(200), 0).unwrap().len() <= 240);
    assert!(safe_filename("会議", i64::MAX).is_err());
}
