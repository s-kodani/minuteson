use super::*;
use crate::minutes_document::{
    tests::{input, item},
    ItemKind,
};

fn sources(count: usize) -> Sources {
    (0..count)
        .map(|i| {
            let mut value = item(ItemKind::Decision);
            value.body = format!("確認事項{i}: {}", "別の作業内容を確認します。".repeat(5));
            (format!("n0-i{i}"), value)
        })
        .collect()
}
#[test]
fn bounded_passes_preserve_all_items_metadata_and_evidence_and_replay_deterministically() {
    let original = sources(40);
    let mut observed = vec![];
    for _ in 0..2 {
        let mut cursor = Cursor::new(original.clone(), 2400, 2048).unwrap();
        let mut trace = vec![];
        while let Some(batch) = cursor.next() {
            trace.push(batch.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>());
            if !cursor.local() {
                assert!(merge::SYSTEM.len() + merge::prompt(batch).unwrap().len() + 256 <= 2400);
                assert!(serde_json::to_vec(&identity(batch)).unwrap().len() + 128 <= 2048);
                assert!(serde_json::to_vec(&merge::schema(batch)).unwrap().len() <= 32 * 1024);
            }
            let parsed = merge::parse(identity(batch), &input(), batch).unwrap();
            cursor.accept(parsed).unwrap();
        }
        assert!(trace.iter().flatten().any(|id| id.starts_with("r1-")));
        let result = cursor.finish().unwrap();
        assert_eq!(result.len(), original.len());
        let before: HashMap<_, _> = original
            .iter()
            .map(|(_, i)| (i.body.clone(), i.clone()))
            .collect();
        let after: HashMap<_, _> = result.into_iter().map(|i| (i.body.clone(), i)).collect();
        assert_eq!(before, after);
        observed.push(trace);
    }
    assert_eq!(observed[0], observed[1]);
}
#[test]
fn transposed_pass_can_reconcile_items_that_were_in_different_initial_windows() {
    let mut source = sources(40);
    for (index, (_, item)) in source.iter_mut().enumerate() {
        item.body = format!(
            "別の作業{}: {}",
            index % 2,
            "点検の作業内容を確認します。".repeat(5)
        );
    }
    let mut cursor = Cursor::new(source.clone(), 2400, 2048).unwrap();
    while let Some(batch) = cursor.next() {
        let mut groups: Vec<(String, Vec<String>)> = vec![];
        for (id, item) in batch {
            if let Some(group) = groups.iter_mut().find(|g| g.0 == item.body) {
                group.1.push(id.clone());
            } else {
                groups.push((item.body.clone(), vec![id.clone()]));
            }
        }
        let raw = json!({"schemaVersion":1,"items":groups.iter().map(|(body,ids)|json!({"body":body,"sourceItemIds":ids})).collect::<Vec<_>>()});
        cursor
            .accept(merge::parse(raw, &input(), batch).unwrap())
            .unwrap();
    }
    let result = cursor.finish().unwrap();
    assert!(result.len() < source.len());
    assert_eq!(
        result
            .iter()
            .map(|i| i.body.clone())
            .collect::<HashSet<_>>(),
        source.iter().map(|(_, i)| i.body.clone()).collect()
    );
    assert!(result
        .iter()
        .all(|i| i.evidence_utterance_ids == source[0].1.evidence_utterance_ids));
}
#[test]
fn incompatible_and_oversized_singletons_are_exact_local_operations_without_invented_fields() {
    let mut source = sources(3);
    source[0].1.body = "長い本文".repeat(1000);
    source[1].1.kind = ItemKind::Todo;
    source[2].1.kind = ItemKind::Summary;
    let mut cursor = Cursor::new(source.clone(), 2400, 2048).unwrap();
    assert!(cursor.local());
    let batch = cursor.next().unwrap();
    cursor
        .accept(merge::parse(identity(batch), &input(), batch).unwrap())
        .unwrap();
    assert_eq!(
        cursor.finish().unwrap(),
        source.into_iter().map(|(_, i)| i).collect::<Vec<_>>()
    );
    let mut duplicate = sources(2);
    duplicate[1].0 = duplicate[0].0.clone();
    assert!(Cursor::new(duplicate, 2400, 2048).is_err());
    assert!(Cursor::new(sources(2), 2400, 2048)
        .unwrap()
        .finish()
        .is_err());
}
#[test]
fn large_parent_outputs_are_interleaved_and_not_capped_to_one_model_call() {
    let mut a = sources(600);
    let mut b = sources(601);
    for (_, i) in a.iter_mut().chain(&mut b) {
        i.body = "確認する".into();
    }
    let parent = vec![
        a.iter().map(|(_, i)| i.clone()).collect(),
        b.iter().map(|(_, i)| i.clone()).collect(),
    ];
    assert!(merge::sources(0, 1, &parent).is_err());
    let source = merge::adaptive_sources(0, 1, &parent).unwrap();
    assert_eq!(source.len(), 1201);
    assert_eq!(source[0].0, "n0-i0");
    assert_eq!(source[1].0, "n1-i0");
    let mut cursor = Cursor::new(source, 60000, 8192).unwrap();
    while let Some(batch) = cursor.next() {
        assert!(batch.len() <= 1000);
        cursor
            .accept(merge::parse(identity(batch), &input(), batch).unwrap())
            .unwrap();
    }
    let result = cursor.finish().unwrap();
    assert_eq!(result.len(), 1201);
    crate::minutes_document::from_generated(&input(), result).unwrap();
}

#[test]
fn model_output_that_repeats_source_ids_across_groups_does_not_advance_the_cursor() {
    let mut cursor = Cursor::new(sources(4), 2400, 2048).unwrap();
    let original = cursor.next().unwrap().clone();
    let mut raw = identity(&original);
    let repeated = raw["items"][0].clone();
    raw["items"].as_array_mut().unwrap().push(repeated);
    assert!(merge::parse(raw, &input(), &original)
        .unwrap_err()
        .contains("重複使用"));
    assert_eq!(cursor.next().unwrap(), &original);
    let parsed = merge::parse(identity(&original), &input(), &original).unwrap();
    cursor.accept(parsed).unwrap();
}

#[test]
#[ignore = "requires MON_LLAMA_COMPLETION and MON_LLAMA_MODEL"]
fn real_llama_bounded_merge_runs_multiple_windows_and_preserves_fixed_evidence() {
    use crate::generation::llama::{run_unit, LlamaConfig};
    use crate::local_engine::Fingerprint;
    use std::{path::Path, time::Duration};
    let root = tempfile::tempdir().unwrap().keep();
    let config = LlamaConfig {
        executable: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_COMPLETION").unwrap()),
            &|| false,
        )
        .unwrap(),
        model: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_MODEL").unwrap()),
            &|| false,
        )
        .unwrap(),
        threads: 4,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    };
    println!("real llama bounded merge fixture: {}", root.display());
    let source = (0..4).map(|i| {
        let mut value = item(ItemKind::Decision);
        value.body = "点検日の確認を決定しました。会議で決まった点検日を確認し、確認した点検日を記録します。".repeat(2);
        (format!("n0-i{i}"), value)
    }).collect();
    let mut cursor = Cursor::new(source, 2400, 2048).unwrap();
    let mut calls = 0;
    let mut crossed = false;
    while let Some(batch) = cursor.next() {
        crossed |= batch.iter().any(|(id, _)| id.starts_with("r1-"));
        let raw = if cursor.local() {
            identity(batch)
        } else {
            let directory = root.join(format!("batch-{calls}"));
            std::fs::create_dir(&directory).unwrap();
            calls += 1;
            run_unit(
                &config,
                &directory,
                merge::SYSTEM,
                &merge::prompt(batch).unwrap(),
                &merge::schema(batch),
                Duration::from_secs(120),
                &|| false,
            )
            .unwrap()
            .unwrap()
        };
        cursor
            .accept(merge::parse(raw, &input(), batch).unwrap())
            .unwrap();
    }
    assert!(calls >= 2);
    assert!(crossed);
    let result = cursor.finish().unwrap();
    assert!(!result.is_empty());
    assert!(result
        .iter()
        .all(|i| i.evidence_utterance_ids == ["utterance"]
            && i.assignee_name.is_none()
            && i.due_date.is_none()));
    crate::minutes_document::from_generated(&input(), result.clone()).unwrap();
    println!(
        "real llama adaptive merge: {calls} calls, {} unverified items, artifacts {}",
        result.len(),
        root.display()
    );
}
