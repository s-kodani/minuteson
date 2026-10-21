//! Native Windows smoke tests; Unix shell/audio fixtures do not substitute here.
use super::*;

#[test]
fn japanese_paths_and_current_meetings_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("会議 データ");
    let id = {
        let storage = Storage::open(root.clone()).unwrap();
        storage.create_meeting("Windowsの会議").unwrap().id
    };
    let storage = Storage::open(root).unwrap();
    let meeting = storage.get_meeting(&id).unwrap().unwrap();
    assert_eq!(meeting.title, "Windowsの会議");
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}

#[test]
fn unknown_database_is_refused_without_replacing_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("meetings.sqlite3");
    let marker = temp.path().join("existing-audio.flac");
    {
        let db = Connection::open(&database).unwrap();
        db.execute_batch("CREATE TABLE unknown_windows_data(value TEXT);")
            .unwrap();
    }
    std::fs::write(&marker, b"SYNTHETIC EXISTING AUDIO").unwrap();
    let before = std::fs::read(&database).unwrap();
    assert!(Storage::open(temp.path().into()).is_err());
    assert_eq!(std::fs::read(database).unwrap(), before);
    assert_eq!(std::fs::read(marker).unwrap(), b"SYNTHETIC EXISTING AUDIO");
}
