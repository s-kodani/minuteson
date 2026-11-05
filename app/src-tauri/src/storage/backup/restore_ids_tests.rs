use super::*;
use crate::storage::draft_splits::{SplitPart, UtteranceSplit};
use crate::storage::drafts::{ManualSpeaker, UtteranceEdit};
use std::cell::Cell;

pub(in crate::storage::backup) fn fixture(storage: &Storage, operation: &str) -> String {
    let meeting = archive::tests::meeting(storage, operation);
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    draft.payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let utterance = &analysis.utterances[0];
    let first = format!("元IDを自然文に保持 {}", utterance.id);
    let text = format!("{first}後半");
    let speaker = format!("manual:{}", Uuid::new_v4());
    draft.payload.manual_speakers.push(ManualSpeaker {
        id: speaker.clone(),
        label: "手動話者".into(),
    });
    draft.payload.utterances.push(UtteranceEdit {
        utterance_id: utterance.id.clone(),
        effective_text: text.clone(),
        speaker_ids: vec![speaker.clone()],
    });
    draft.payload.utterance_splits.push(UtteranceSplit {
        source_utterance_id: utterance.id.clone(),
        source_effective_text: text,
        parts: vec![
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: 0,
                end_us: 500_000,
                effective_text: first,
                speaker_ids: vec![speaker.clone()],
            },
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: 500_000,
                end_us: 1_000_000,
                effective_text: "後半".into(),
                speaker_ids: vec![speaker],
            },
        ],
    });
    storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    super::super::super::current_minutes_fixture::candidate(storage, &meeting);

    meeting
}

fn read(storage: &Storage, output: &Path) -> VerifiedArchive {
    let path = output.join("source.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    archive::read(
        &mut File::open(path).unwrap(),
        output,
        &crate::test_support::never_cancelled,
    )
    .unwrap()
}

#[test]
fn relational_rules_cover_every_identifier_and_fk_in_the_trusted_schema() {
    let db = portable::schema().unwrap();
    let tables = portable::tables(&db).unwrap();
    assert_eq!(tables.len(), portable::TABLES.len());
    for table in &tables {
        let bindings = rules(&db, table).unwrap();
        for column in &table.columns {
            if column.name == "id" || column.name.ends_with("_id") {
                assert!(
                    bindings.contains_key(&column.name),
                    "{}.{}",
                    table.name,
                    column.name
                );
            }
        }
    }
    let mut changed = tables[0].clone();
    changed.columns.push(portable::Column {
        name: "future_reference_id".into(),
        data_type: "TEXT".into(),
    });
    assert!(rules(&db, &changed).unwrap_err().contains("未対応"));
    assert!(entity_table("injected; DROP TABLE meetings", "id").is_err());
}

#[test]
fn all_relational_ids_are_new_and_scoped_while_sources_and_live_data_stay_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = fixture(&storage, "import-a");
    let b = fixture(&storage, "import-b");
    let source = read(&storage, output.path());
    let batch = Uuid::new_v4().to_string();
    let plan = RestoreIds::prepare(
        &source,
        output.path(),
        &batch,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(plan.batch_id, batch);
    assert_eq!(plan.archive_id, source.manifest.archive_id);
    let new_a = plan.get("meetings", "", &a).unwrap();
    assert_ne!(new_a, a);
    assert_ne!(new_a, plan.get("meetings", "", &b).unwrap());
    let accidental_old: i64 = plan
        .database
        .query_row(
            "SELECT COUNT(*) FROM mappings m JOIN reserved_ids r ON r.id=m.new_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(accidental_old, 0);
    // This tests relational coherence, not runnable restored JSON/hash state.
    let projected = portable::schema().unwrap();
    let triggers: Vec<String> = projected
        .prepare("SELECT name FROM sqlite_schema WHERE type='trigger'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for trigger in triggers {
        projected
            .execute_batch(&format!("DROP TRIGGER \"{trigger}\";"))
            .unwrap();
    }
    projected
        .execute_batch("BEGIN; PRAGMA defer_foreign_keys=ON;")
        .unwrap();
    for table in portable::tables(&source.database).unwrap() {
        let bindings = rules(&source.database, &table).unwrap();
        let mut query = source
            .database
            .prepare(&owners_sql(&table.name).unwrap())
            .unwrap();
        let mut rows = query.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let rowid: i64 = row.get(0).unwrap();
            let meeting: Option<String> = row.get(1).unwrap();
            let mapped = plan
                .mapped_row(
                    &source.database,
                    &table,
                    &bindings,
                    rowid,
                    meeting.as_deref().unwrap_or(""),
                )
                .unwrap();
            assert_eq!(mapped, plan.relational_row(&table.name, rowid).unwrap());
            let columns = table
                .columns
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect::<Vec<_>>()
                .join(",");
            let placeholders = vec!["?"; mapped.len()].join(",");
            projected
                .execute(
                    &format!(
                        "INSERT INTO {}({columns}) VALUES({placeholders})",
                        table.name
                    ),
                    rusqlite::params_from_iter(&mapped),
                )
                .unwrap();
            for (column, value) in table.columns.iter().zip(&mapped) {
                if let Value::Text(new) = value {
                    if bindings.contains_key(&column.name)
                        && column.name != "scope_id"
                        && table.name != "vocabulary_scopes"
                    {
                        let old: String = source
                            .database
                            .query_row(
                                &format!(
                                    "SELECT \"{}\" FROM \"{}\" WHERE rowid=?1",
                                    column.name, table.name
                                ),
                                params![rowid],
                                |r| r.get(0),
                            )
                            .unwrap();
                        assert_ne!(*new, old, "{}.{}", table.name, column.name);
                    }
                }
            }
        }
    }
    projected.execute_batch("COMMIT;").unwrap();
    assert!(projected
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
    assert!(plan
        .relational_row("meetings; DELETE FROM meetings", 1)
        .is_err());
    let old_utterance: String = source
        .database
        .query_row(
            "SELECT id FROM utterances WHERE meeting_id=?1 ORDER BY rowid LIMIT 1",
            params![a],
            |r| r.get(0),
        )
        .unwrap();
    assert!(plan.get("utterances", &b, &old_utterance).is_err());
    assert!(plan.get("utterances", &a, "undefined").is_err());
    assert!(plan.database.execute("DELETE FROM mappings", []).is_err());
    assert!(source.database.execute("DELETE FROM meetings", []).is_err());
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
    assert_eq!(source.manifest.meetings.len(), 2);
    assert!(storage.get_meeting(&new_a).unwrap().is_none());
    let another = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    assert_ne!(another.get("meetings", "", &a).unwrap(), new_a);
}

#[test]
fn cancellation_and_invalid_batch_remove_only_owned_planning_files() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "import");
    let source = read(&storage, output.path());
    let unrelated = output.path().join("unrelated.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    let before = std::fs::read_dir(output.path()).unwrap().count();
    assert!(RestoreIds::prepare(
        &source,
        output.path(),
        "not-a-batch",
        &crate::test_support::never_cancelled
    )
    .is_err());
    let calls = Cell::new(0);
    assert!(
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
            calls.set(calls.get() + 1);
            calls.get() > 80
        })
        .is_err()
    );
    assert!(calls.get() > 80);
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    let plan = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let path = plan.stage.path().to_path_buf();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path.join("identities.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(plan);
    assert!(!path.exists());
}

#[test]
fn transcript_references_from_other_meetings_are_rejected_including_manual_edits() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = fixture(&storage, "a");
    let b = fixture(&storage, "b");
    let source = read(&storage, output.path());
    let plan = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let mut payload = storage.load_transcript_draft(&a).unwrap().unwrap().payload;
    let other = storage
        .load_current_analysis(&b)
        .unwrap()
        .unwrap()
        .utterances[0]
        .id
        .clone();
    payload.utterances[0].utterance_id = other;
    let before = payload.clone();
    assert!(plan.transcript(&a, &mut payload).is_err());
    assert_eq!(payload, before);
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
}
