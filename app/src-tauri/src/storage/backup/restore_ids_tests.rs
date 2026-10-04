use super::*;
use crate::storage::draft_splits::{SplitPart, UtteranceSplit};
use crate::storage::drafts::{ManualSpeaker, UtteranceEdit};
use std::cell::Cell;

pub(in crate::storage::backup) fn fixture(storage: &Storage, operation: &str) -> String {
    let meeting = archive::tests::meeting(storage, operation);
    let mut draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    draft.payload.schema_version = 3;
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
    let revision = super::super::super::minutes_ui::tests::candidate(storage, &meeting);
    storage
        .adopt_minutes(super::super::super::minutes_ui::tests::request(
            storage, &meeting, &revision,
        ))
        .unwrap();
    let mut minutes = storage.create_minutes_draft(&meeting, &revision).unwrap();
    for text in ["手動追記1", "手動追記2"] {
        minutes.payload.blocks.push(Block::Paragraph {
            item_id: None,
            content: vec![Inline::text(text)],
        });
        minutes = storage
            .save_minutes_draft(&minutes.id, minutes.lock_version, minutes.payload)
            .unwrap();
    }
    meeting
}

fn read(storage: &Storage, output: &Path) -> VerifiedArchive {
    let path = output.join("source.monbackup");
    storage.create_backup_archive(&path, &|| false).unwrap();
    archive::read(&mut File::open(path).unwrap(), output, &|| false).unwrap()
}

#[test]
fn relational_rules_cover_every_identifier_and_fk_in_the_trusted_schema() {
    let db = portable::schema(MIGRATIONS.len()).unwrap();
    let tables = portable::tables(&db).unwrap();
    assert_eq!(tables.len(), 31);
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
    let plan = RestoreIds::prepare(&source, output.path(), &batch, &|| false).unwrap();
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
    let projected = portable::schema(MIGRATIONS.len()).unwrap();
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
fn drafts_splits_undo_and_rich_document_references_share_the_new_identity_without_text_replacement()
{
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "import");
    let source = read(&storage, output.path());
    let plan = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let original = storage
        .load_transcript_draft(&meeting)
        .unwrap()
        .unwrap()
        .payload;
    let mut mapped = original.clone();
    plan.transcript(&meeting, &mut mapped).unwrap();
    assert_ne!(mapped, original);
    assert_eq!(
        mapped.manual_speakers[0].id,
        plan.get("speaker_lineage", &meeting, &original.manual_speakers[0].id)
            .unwrap()
    );
    assert_eq!(
        mapped.utterance_splits[0].parts[0].id,
        plan.get(
            "utterance_lineage",
            &meeting,
            &original.utterance_splits[0].parts[0].id
        )
        .unwrap()
    );
    assert_eq!(
        mapped.utterance_splits[0].parts[0].effective_text,
        original.utterance_splits[0].parts[0].effective_text
    );
    assert_eq!(
        mapped.utterance_splits[0].source_effective_text,
        original.utterance_splits[0].source_effective_text
    );
    assert_eq!(
        mapped.utterances[0].speaker_ids[0],
        mapped.manual_speakers[0].id
    );
    assert_eq!(
        mapped.utterance_splits[0].parts[0].speaker_ids[0],
        mapped.manual_speakers[0].id
    );
    let json: String = source
        .database
        .query_row(
            "SELECT payload_json FROM edit_operations LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut history: TranscriptDraftPayload = serde_json::from_str(&json).unwrap();
    plan.transcript(&meeting, &mut history).unwrap();
    assert_eq!(history, mapped);
    let json: String = source
        .database
        .query_row(
            "SELECT effective_document_json FROM minutes_revisions WHERE meeting_id=?1 LIMIT 1",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap();
    let mut content: MinutesContent = serde_json::from_str(&json).unwrap();
    let old_content = content.clone();
    let inline = Inline::Evidence {
        minutes_item_id: content.items[0].id.clone(),
        evidence_link_id: content.evidence[0].id.clone(),
    };
    // Exercise all rich-text containers, not just the generated paragraph.
    content.blocks = vec![
        Block::Heading {
            level: 1,
            item_id: Some(content.items[0].id.clone()),
            content: vec![Inline::text(&content.items[0].id), inline.clone()],
        },
        Block::List {
            ordered: false,
            items: vec![crate::minutes_document::TextBlock {
                item_id: Some(content.items[0].id.clone()),
                content: vec![inline.clone()],
            }],
        },
        Block::Table {
            headers: vec![vec![inline.clone()]],
            rows: vec![crate::minutes_document::TableRow {
                item_id: Some(content.items[0].id.clone()),
                cells: vec![vec![inline]],
            }],
        },
    ];
    plan.minutes(&meeting, &mut content).unwrap();
    assert_ne!(content.items[0].id, old_content.items[0].id);
    assert_eq!(content.items[0].body, old_content.items[0].body);
    assert_eq!(
        content.evidence[0].quote_text,
        old_content.evidence[0].quote_text
    );
    assert_eq!(content.evidence[0].minutes_item_id, content.items[0].id);
    if let Block::Heading {
        item_id,
        content: parts,
        ..
    } = &content.blocks[0]
    {
        assert_eq!(item_id.as_ref(), Some(&content.items[0].id));
        assert_eq!(parts[0], Inline::text(&old_content.items[0].id));
        assert_eq!(
            parts[1],
            Inline::Evidence {
                minutes_item_id: content.items[0].id.clone(),
                evidence_link_id: content.evidence[0].id.clone()
            }
        );
    } else {
        panic!("heading missing");
    }
    let (before, after): (String, String) = source
        .database
        .query_row(
            "SELECT before_hash,after_hash FROM edit_operations LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_ne!(
        super::super::super::drafts::draft_hash(&serde_json::to_string(&mapped).unwrap()),
        after
    );
    assert_ne!(before, after); // Hash reconstruction belongs to the next stage.
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
    assert!(RestoreIds::prepare(&source, output.path(), "not-a-batch", &|| false).is_err());
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
fn typed_document_references_from_other_meetings_are_rejected_including_manual_edits() {
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
    let text: String = source
        .database
        .query_row(
            "SELECT effective_document_json FROM minutes_revisions WHERE meeting_id=?1",
            params![a],
            |r| r.get(0),
        )
        .unwrap();
    let mut content: MinutesContent = serde_json::from_str(&text).unwrap();
    content.evidence[0].analysis_revision_id =
        storage.load_current_analysis(&b).unwrap().unwrap().id;
    let before = content.clone();
    assert!(plan.minutes(&a, &mut content).is_err());
    assert_eq!(content, before);
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
}

#[test]
fn private_plan_rejects_all_four_dependency_cycles_without_modifying_live_data() {
    for domain in [
        "analysis_revisions",
        "minutes_revisions",
        "processing_jobs",
        "audio_tracks",
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        let meeting = fixture(&storage, "import");
        let source = read(&storage, output.path());
        // Mutate only the operation-owned validation copy to model rehashed input.
        source
            .database
            .execute_batch("PRAGMA query_only=OFF;")
            .unwrap();
        match domain {
            "analysis_revisions" | "minutes_revisions" => {
                source
                    .database
                    .execute(&format!("UPDATE {domain} SET parent_revision_id=id"), [])
                    .unwrap();
            }
            "processing_jobs" => {
                source
                    .database
                    .execute("UPDATE processing_jobs SET retry_of_job_id=id", [])
                    .unwrap();
            }
            _ => {
                let track: String = source
                    .database
                    .query_row("SELECT id FROM audio_tracks LIMIT 1", [], |r| r.get(0))
                    .unwrap();
                source.database.execute("INSERT INTO audio_tracks(id,meeting_id,import_id,kind,format,sample_rate,channels,state,created_at,updated_at) SELECT 'extra-track',meeting_id,import_id,'processed',format,sample_rate,channels,state,created_at,updated_at FROM audio_tracks WHERE id=?1",params![track]).unwrap();
                source
                    .database
                    .execute(
                        "INSERT INTO track_inputs VALUES(?1,?2,'extra-track','fixture')",
                        params![meeting, track],
                    )
                    .unwrap();
                source
                    .database
                    .execute(
                        "INSERT INTO track_inputs VALUES(?1,'extra-track',?2,'fixture')",
                        params![meeting, track],
                    )
                    .unwrap();
            }
        }
        source
            .database
            .execute_batch("PRAGMA query_only=ON;")
            .unwrap();
        let before = std::fs::read_dir(output.path()).unwrap().count();
        let error =
            RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
                false
            })
            .err()
            .unwrap();
        assert!(error.contains("循環") && error.contains(domain), "{error}");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
        assert_eq!(storage.list_meetings().unwrap().len(), 1);
        assert!(storage
            .verify_analysis_content_hash(
                &storage.load_current_analysis(&meeting).unwrap().unwrap().id
            )
            .unwrap());
    }
}

#[test]
fn a_long_parent_chain_is_planned_iteratively_and_every_parent_precedes_its_child() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "import");
    let source = read(&storage, output.path());
    source
        .database
        .execute_batch("PRAGMA query_only=OFF; BEGIN;")
        .unwrap();
    let original: String = source
        .database
        .query_row(
            "SELECT current_analysis_id FROM meetings WHERE id=?1",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap();
    let number: i64 = source
        .database
        .query_row("SELECT MAX(revision_no) FROM analysis_revisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    let mut parent = original.clone();
    for index in 0..3_000 {
        let id = format!("deep-{index}");
        source.database.execute("INSERT INTO analysis_revisions(id,meeting_id,parent_revision_id,revision_no,parent_content_hash,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) SELECT ?1,meeting_id,?2,?3,content_hash,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at FROM analysis_revisions WHERE id=?4",params![id,parent,number+index+1,original]).unwrap();
        parent = id;
    }
    source
        .database
        .execute_batch("COMMIT; PRAGMA query_only=ON;")
        .unwrap();
    let plan = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let invalid:i64=plan.database.query_row("SELECT COUNT(*) FROM publication_order child JOIN publication_order parent ON child.domain=parent.domain WHERE child.domain='analysis_revisions' AND child.old_id LIKE 'deep-%' AND parent.old_id='deep-' || (CAST(substr(child.old_id,6) AS INTEGER)-1) AND child.position<=parent.position",[],|r|r.get(0)).unwrap();
    assert_eq!(invalid, 0);
    let count: i64 = plan
        .database
        .query_row(
            "SELECT COUNT(*) FROM publication_order WHERE domain='analysis_revisions'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3_002);
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}

#[test]
fn preparation_rejects_a_cross_meeting_undo_reference_and_keeps_the_source_archive() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let a = fixture(&storage, "a");
    let b = fixture(&storage, "b");
    let source = read(&storage, output.path());
    let source_path = output.path().join("source.monbackup");
    let original_hash = digest_file(&source_path, &|| false).unwrap();
    let text:String=source.database.query_row("SELECT e.payload_json FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id WHERE d.meeting_id=?1 AND d.target_kind='transcript'",params![a],|r|r.get(0)).unwrap();
    let mut payload: TranscriptDraftPayload = serde_json::from_str(&text).unwrap();
    payload.utterances[0].utterance_id = storage
        .load_current_analysis(&b)
        .unwrap()
        .unwrap()
        .utterances[0]
        .id
        .clone();
    source
        .database
        .execute_batch("PRAGMA query_only=OFF;")
        .unwrap();
    let inverse = serde_json::to_string(&payload).unwrap();
    source.database.execute("UPDATE edit_operations SET inverse_json=?1,before_hash=?3 WHERE draft_id IN (SELECT id FROM working_drafts WHERE meeting_id=?2 AND target_kind='transcript')",params![inverse,a,draft_hash("transcript",&inverse).unwrap()]).unwrap();
    source
        .database
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    let before = std::fs::read_dir(output.path()).unwrap().count();
    assert!(
        RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
            false
        })
        .err()
        .unwrap()
        .contains("別会議")
    );
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
    assert_eq!(digest_file(&source_path, &|| false).unwrap(), original_hash);
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
}

#[test]
fn rebuilding_drafts_and_undo_preserves_the_chain_and_text_with_new_reference_hashes() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "import");
    let source = read(&storage, output.path());
    let plan = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let tables = portable::tables(&source.database).unwrap();
    let value = |table: &str, values: &[Value], name: &str| match &values[tables
        .iter()
        .find(|t| t.name == table)
        .unwrap()
        .columns
        .iter()
        .position(|c| c.name == name)
        .unwrap()]
    {
        Value::Text(s) => s.clone(),
        _ => panic!("wrong type"),
    };
    let mut query = source
        .database
        .prepare("SELECT rowid,id,target_kind,content_hash FROM working_drafts ORDER BY id")
        .unwrap();
    let mut rows = query.query([]).unwrap();
    let mut counts = std::collections::BTreeMap::new();
    while let Some(row) = rows.next().unwrap() {
        let rowid: i64 = row.get(0).unwrap();
        let id: String = row.get(1).unwrap();
        let kind: String = row.get(2).unwrap();
        let original_hash: String = row.get(3).unwrap();
        let mapped = plan.draft_row("working_drafts", rowid).unwrap();
        let hash = value("working_drafts", &mapped, "content_hash");
        let payload = value("working_drafts", &mapped, "payload_json");
        assert_eq!(hash, draft_hash(&kind, &payload).unwrap());
        assert_ne!(hash, original_hash);
        let mut query = source
            .database
            .prepare("SELECT rowid FROM edit_operations WHERE draft_id=?1 ORDER BY sequence")
            .unwrap();
        let mut edits = query.query(params![id]).unwrap();
        let mut previous: Option<String> = None;
        let mut count = 0;
        while let Some(edit) = edits.next().unwrap() {
            let mapped = plan
                .draft_row("edit_operations", edit.get(0).unwrap())
                .unwrap();
            let payload = value("edit_operations", &mapped, "payload_json");
            let inverse = value("edit_operations", &mapped, "inverse_json");
            let before = value("edit_operations", &mapped, "before_hash");
            let after = value("edit_operations", &mapped, "after_hash");
            assert_eq!(before, draft_hash(&kind, &inverse).unwrap());
            assert_eq!(after, draft_hash(&kind, &payload).unwrap());
            if let Some(previous) = previous {
                assert_eq!(previous, before);
            }
            previous = Some(after);
            count += 1;
        }
        assert_eq!(previous, Some(hash));
        counts.insert(kind, count);
    }
    assert_eq!(counts.get("minutes"), Some(&2));
    assert_eq!(counts.get("transcript"), Some(&1));
    assert!(plan.draft_row("processing_jobs", 1).is_err());
    assert!(plan.draft_json("unknown", "", "{}").is_err());
}

#[test]
fn modified_draft_hash_and_broken_undo_chain_are_rejected_before_planning() {
    for corruption in ["current", "payload", "inverse", "sequence", "chain"] {
        let root = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        fixture(&storage, "import");
        let source = read(&storage, output.path());
        source
            .database
            .execute_batch("PRAGMA query_only=OFF;")
            .unwrap();
        let sql=match corruption {
            "current"=>"UPDATE working_drafts SET content_hash=replace(hex(zeroblob(32)),'0','f')",
            "payload"=>"UPDATE edit_operations SET after_hash=replace(hex(zeroblob(32)),'0','f')",
            "inverse"=>"UPDATE edit_operations SET before_hash=replace(hex(zeroblob(32)),'0','f')",
            "sequence"=>"UPDATE edit_operations SET sequence=sequence+100",
            _=>"DELETE FROM edit_operations WHERE sequence=1 AND draft_id IN (SELECT id FROM working_drafts WHERE target_kind='minutes')",
        };
        source.database.execute(sql, []).unwrap();
        source
            .database
            .execute_batch("PRAGMA query_only=ON;")
            .unwrap();
        let before = std::fs::read_dir(output.path()).unwrap().count();
        assert!(
            RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
                false
            })
            .is_err(),
            "{corruption}"
        );
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
        assert_eq!(storage.list_meetings().unwrap().len(), 1);
    }
}
