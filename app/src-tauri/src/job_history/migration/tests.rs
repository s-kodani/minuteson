use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    initialize(&db, 0).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db
}
fn id(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}
fn row(kind: &str, operation: &str, meeting: &str, state: &str, at: i64) -> Entry {
    let mut e = Entry::new(kind, operation, "会議", Some(meeting.into()), state);
    e.accepted_at = Some(at - 10);
    e.observed_at = at;
    e.finished_at = (state == "succeeded" || state == "failed").then_some(at);
    e.detail = Some(format!("{state}:{at}"));
    e
}
fn insert(db: &Connection, e: &Entry) {
    db.execute(
        "INSERT INTO history VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            e.id,
            e.kind,
            e.target,
            e.meeting_id,
            e.state,
            e.accepted_at,
            e.finished_at,
            e.observed_at,
            e.detail,
            e.active()
        ],
    )
    .unwrap();
}
fn rows(db: &Connection) -> Vec<Entry> {
    db.prepare("SELECT id,kind,target,meeting_id,state,accepted_at,finished_at,observed_at,detail FROM history ORDER BY id").unwrap()
        .query_map([], read).unwrap().collect::<Result<_, _>>().unwrap()
}
fn version(db: &Connection) -> i64 {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn merges_retries_into_the_first_start_and_latest_result_and_skips_completed_migration() {
    let db = fixture();
    let session = id(1);
    let meeting = id(2);
    let save = row("recording_save", &session, &meeting, "running", 100);
    let recovery = row(
        "recording_recovery",
        &format!("{}:{session}", id(3)),
        &meeting,
        "failed",
        200,
    );
    let retry = row(
        "recording_recovery",
        &format!("{}:{session}", id(4)),
        &meeting,
        "succeeded",
        300,
    );
    for e in [&save, &recovery, &retry] {
        insert(&db, e);
    }
    let unrelated = row("transcription", "existing", &meeting, "succeeded", 100);
    insert(&db, &unrelated);
    initialize(&db, 1).unwrap();
    let records = rows(&db);
    assert_eq!(version(&db), 2);
    assert_eq!(records.len(), 2);
    let merged = records.iter().find(|e| e.kind == "recording_save").unwrap();
    assert_eq!(merged.id, format!("recording_save:{session}"));
    assert_eq!(merged.accepted_at, save.accepted_at);
    assert_eq!(merged.state, "succeeded");
    assert_eq!(merged.observed_at, 300);
    assert_eq!(merged.finished_at, Some(300));
    assert_eq!(merged.detail, retry.detail);
    initialize(&db, 2).unwrap();
    assert_eq!(rows(&db), records);
    assert_eq!(
        records.iter().find(|e| e.kind == "transcription"),
        Some(&unrelated)
    );
}

#[test]
fn equal_observations_prefer_confirmed_results_then_original_save_and_other_meetings_stay_separate()
{
    let db = fixture();
    let session = id(10);
    let meeting = id(11);
    insert(
        &db,
        &row("recording_save", &session, &meeting, "interrupted", 100),
    );
    insert(
        &db,
        &row(
            "recording_recovery",
            &format!("{}:{session}", id(12)),
            &meeting,
            "succeeded",
            100,
        ),
    );
    let session2 = id(20);
    let meeting2 = id(21);
    let saved = row("recording_save", &session2, &meeting2, "failed", 100);
    insert(&db, &saved);
    insert(
        &db,
        &row(
            "recording_recovery",
            &format!("{}:{session2}", id(22)),
            &meeting2,
            "succeeded",
            100,
        ),
    );
    initialize(&db, 1).unwrap();
    let records = rows(&db);
    assert_eq!(records.len(), 2);
    assert_eq!(
        records
            .iter()
            .find(|e| e.meeting_id.as_deref() == Some(meeting.as_str()))
            .unwrap()
            .state,
        "succeeded"
    );
    assert_eq!(
        records
            .iter()
            .find(|e| e.meeting_id.as_deref() == Some(meeting2.as_str()))
            .unwrap(),
        &saved
    );
}

#[test]
fn invalid_legacy_identifiers_or_cross_meeting_session_collisions_roll_back_without_changing_any_row(
) {
    for invalid in ["bad-run", "missing-session", "other-meeting"] {
        let db = fixture();
        let session = id(30);
        let meeting = id(31);
        insert(
            &db,
            &row("recording_save", &session, &meeting, "running", 100),
        );
        let (operation, second_meeting) = match invalid {
            "bad-run" => (format!("invalid:{session}"), meeting.clone()),
            "missing-session" => (id(32), meeting.clone()),
            _ => (format!("{}:{session}", id(32)), id(33)),
        };
        insert(
            &db,
            &row(
                "recording_recovery",
                &operation,
                &second_meeting,
                "succeeded",
                200,
            ),
        );
        let before = rows(&db);
        assert!(initialize(&db, 1).is_err(), "{invalid}");
        assert_eq!(version(&db), 1);
        assert_eq!(rows(&db), before);
    }
}

#[test]
fn write_failure_rolls_back_deletions_and_version_before_a_successful_retry() {
    let db = fixture();
    let session = id(40);
    insert(
        &db,
        &row(
            "recording_recovery",
            &format!("{}:{session}", id(41)),
            &id(42),
            "succeeded",
            100,
        ),
    );
    db.execute_batch("CREATE TRIGGER migration_failure BEFORE INSERT ON history BEGIN SELECT RAISE(ABORT,'synthetic migration write failure'); END;").unwrap();
    let before = rows(&db);
    assert!(initialize(&db, 1).is_err());
    assert_eq!(version(&db), 1);
    assert_eq!(rows(&db), before);
    db.execute_batch("DROP TRIGGER migration_failure").unwrap();
    initialize(&db, 1).unwrap();
    assert_eq!(version(&db), 2);
    assert_eq!(rows(&db).len(), 1);
}

#[test]
fn migrated_history_keeps_filter_paging_retention_and_meeting_deletion() {
    use super::super::{query, Query, RETENTION_MS};
    let db = fixture();
    let meeting = id(50);
    for n in 1..=3 {
        insert(
            &db,
            &row(
                "recording_recovery",
                &format!("{}:{}", id(60), id(60 + n)),
                &meeting,
                "succeeded",
                1000,
            ),
        );
    }
    insert(
        &db,
        &row("recording_save", &id(70), &meeting, "failed", 999),
    );
    initialize(&db, 1).unwrap();
    let meetings = BTreeMap::from([(meeting.clone(), "現在の会議名".into())]);
    let page = query(
        &db,
        Query {
            kind: Some("recording_save".into()),
            limit: Some(2),
            ..Default::default()
        },
        &meetings,
        RETENTION_MS + 1000,
    )
    .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page
        .items
        .iter()
        .all(|e| e.kind == "recording_save" && e.state == "succeeded"));
    let next = query(
        &db,
        Query {
            kind: Some("recording_save".into()),
            cursor: page.next_cursor,
            ..Default::default()
        },
        &meetings,
        RETENTION_MS + 1000,
    )
    .unwrap();
    assert_eq!(next.items.len(), 1);
    assert!(next.next_cursor.is_none());
    assert!(
        query(&db, Query::default(), &BTreeMap::new(), RETENTION_MS + 1000)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(rows(&db).is_empty());
}

#[test]
fn reopen_migrates_the_persistent_database_once_and_retains_the_completed_conversion() {
    use super::super::open;
    let dir = tempfile::tempdir().unwrap();
    {
        let db = open(dir.path()).unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        insert(
            &db,
            &row(
                "recording_recovery",
                &format!("{}:{}", id(80), id(81)),
                &id(82),
                "succeeded",
                100,
            ),
        );
    }
    let before;
    {
        let db = open(dir.path()).unwrap();
        assert_eq!(version(&db), 2);
        before = rows(&db);
        db.execute_batch("CREATE TRIGGER reject_repeat_conversion BEFORE DELETE ON history BEGIN SELECT RAISE(ABORT,'conversion repeated'); END;").unwrap();
    }
    let db = open(dir.path()).unwrap();
    assert_eq!(version(&db), 2);
    assert_eq!(rows(&db), before);
}
