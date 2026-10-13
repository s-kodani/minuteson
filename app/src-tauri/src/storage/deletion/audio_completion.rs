//! Atomic availability change, retaining IDs, times, text and import provenance.
//! A completed receipt is checked as audio, never ignored by a JSON route hint.
use super::super::{active, Prepared};
use super::{Phase, Receipt};
use crate::{
    analysis_worker::data_maintenance::DataMaintenanceLease,
    storage::{
        deletion::{
            database::{self, Table},
            fs::RootAnchor,
        },
        Storage,
    },
};
use rusqlite::{params, Connection, TransactionBehavior};

pub(super) fn validate_tables(saved: &Prepared, tables: &[Table]) -> Result<(), String> {
    let names = ["audio_chunks", "audio_tracks", "file_assets"];
    if tables.len() != 3
        || tables.iter().zip(names).any(|(t, n)| {
            t.name != n
                || !super::super::hash_valid(&t.sha256)
                || t.rows
                    != if n == "file_assets" {
                        saved.snapshot.records.audio_assets.len() as u64
                    } else {
                        saved
                            .snapshot
                            .scope
                            .tables
                            .iter()
                            .find(|t| t.name == n)
                            .map_or(u64::MAX, |t| t.rows)
                    }
        })
    {
        return Err("音声利用不可の履歴指紋・件数が不正です".into());
    }
    Ok(())
}
pub(in crate::storage::deletion::audio::intent) fn verify_available_state(
    db: &Connection,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    verify_available_state_inner(db, r, check)?;
    if r.saved.header.origin.is_some() {
        super::super::reconfirmation::Source::for_prepared(db, &r.saved, check)?;
    }
    Ok(())
}
pub(in crate::storage::deletion::audio::intent) fn verify_available_state_inner(
    db: &Connection,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    if !r.completed() {
        return Err("音声削除は完了していません".into());
    }
    let expected = r
        .progress
        .final_audio
        .as_ref()
        .ok_or("音声利用不可の証拠がありません")?;
    validate_tables(&r.saved, expected)?;
    if database::audio_tables(db, r.saved.meeting(), check)? != *expected {
        return Err("削除済み音声の資産・区間・トラック履歴が変更されています".into());
    }
    let available:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state!='missing') OR EXISTS(SELECT 1 FROM audio_chunks WHERE meeting_id=?1 AND state!='missing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND state!='failed')",params![r.saved.meeting()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if available {
        return Err("削除済み音声の利用状態が復活しています".into());
    }
    super::super::super::cancel(check)
}
// Presence/identity only, not a body scan or an execution permission. Text
// artifacts and later minutes jobs may change normally after completion.
pub(super) fn absent(
    db: &Connection,
    anchor: &RootAnchor,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    if !anchor.matches_history_root(
        db,
        r.saved.operation(),
        &r.saved.snapshot.manifest.root,
        &r.saved.snapshot.manifest.database,
    )? {
        return Err("音声削除記録の保存領域・DB実体が一致しません".into());
    }
    let Some(meetings) = anchor.directory_ref()?.child("meetings")? else {
        return Ok(true);
    };
    for path in &r.historical_paths {
        super::super::super::cancel(check)?;
        let Some(mut dir) = meetings.child(r.saved.meeting())? else {
            continue;
        };
        let mut prefix = format!("meetings/{}", r.saved.meeting());
        let root = r
            .plan
            .manifest
            .entries
            .iter()
            .find(|e| e.path == prefix)
            .and_then(|e| e.proof.as_ref())
            .ok_or("会議ルートの確認実体がありません")?;
        anchor.verify_history_directory(db, r.saved.operation(), &prefix, &dir, &root.identity)?;
        let rest = path
            .strip_prefix(&format!("{prefix}/"))
            .ok_or("音声対象が会議外です")?;
        let (parents, leaf) = rest.rsplit_once('/').unwrap_or(("", rest));
        let mut missing = false;
        for component in parents.split('/').filter(|s| !s.is_empty()) {
            prefix = format!("{prefix}/{component}");
            let Some(child) = dir.child(component)? else {
                missing = true;
                break;
            };
            let expected = r
                .plan
                .manifest
                .entries
                .iter()
                .find(|e| e.path == prefix)
                .and_then(|e| e.proof.as_ref())
                .ok_or("音声の親を別の実体として採用しません")?;
            anchor.verify_history_directory(
                db,
                r.saved.operation(),
                &prefix,
                &child,
                &expected.identity,
            )?;
            dir = child;
        }
        if !missing && dir.optional_kind(leaf)?.is_some() {
            return Ok(false);
        }
    }
    anchor.verify()?;
    super::super::super::cancel(check)?;
    Ok(true)
}
pub(in crate::storage::deletion) fn inspect_one(
    db: &Connection,
    anchor: &RootAnchor,
    operation: &str,
) -> Result<Option<&'static str>, String> {
    let checked = (|| {
        let r = Receipt::load(db, operation, &|| false)?;
        verify_available_state(db, &r, &|| false)?;
        if !anchor.matches_history_root(
            db,
            operation,
            &r.saved.snapshot.manifest.root,
            &r.saved.snapshot.manifest.database,
        )? {
            return Err("音声削除記録の保存領域が一致しません".to_string());
        }
        match absent(db, anchor, &r, &|| false) {
            Ok(true) => Ok(None),
            Ok(false) => Ok(Some("audio_completed_scope_reappeared")),
            Err(_) => Ok(Some("audio_completed_scope_uninspectable")),
        }
    })();
    // Invalid typed data stays fenced; a subtype or completed label alone
    // never exempts a row from startup inspection.
    Ok(checked.unwrap_or(Some("audio_completed_receipt_invalid")))
}
pub(in crate::storage) fn deleted(db: &Connection, meeting: &str) -> Result<bool, String> {
    let has_table: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='maintenance_operations')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !has_table {
        return Err("現行DBの音声削除台帳が欠落しています。音声を利用しません".into());
    }
    // Also route a restored header claiming this meeting when its nullable FK
    // was detached. The raw claim is NOT authority: strict load must match it
    // to the FK and ledger, or source use fails closed. Model receipts do not
    // carry this top-level meeting field. Unknown subtypes are never exempted.
    let ids=db.prepare("SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' AND (meeting_id=?1 OR json_extract(target_manifest_json,'$.meeting')=?1) ORDER BY id LIMIT 257").map_err(|e|e.to_string())?.query_map(params![meeting],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if ids.len() > 256 {
        return Err("音声削除履歴が安全上限を超えています".into());
    }
    for id in &ids {
        if crate::storage::deletion::audio::restored::hinted(db, id)? {
            crate::storage::deletion::audio::restored::deleted(db, id, meeting)?;
            continue;
        }
        let r = Receipt::load(db, id, &|| false)?;
        if r.saved.meeting() != meeting {
            return Err("音声削除の会議が一致しません".into());
        }
        verify_available_state(db, &r, &|| false)?;
    }
    Ok(!ids.is_empty())
}

// Read-only backup admission. The image carries the original journal but has
// a different inode, so absence is checked against the live, fenced anchor.
// Returned meeting IDs are historical facts, never a portable removal permit.
pub(in crate::storage) fn backup_deleted(
    storage: &Storage,
    db: &Connection,
    check: &impl Fn() -> bool,
) -> Result<std::collections::BTreeSet<String>, String> {
    super::super::super::cancel(check)?;
    crate::storage::deletion::audio_supersession::verify_detached_kept(
        db,
        &storage.managed_anchor,
        check,
    )?;
    let ids = db.prepare("SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' AND (meeting_id IS NOT NULL OR json_type(target_manifest_json,'$.meeting') IS NOT NULL) ORDER BY id LIMIT 10001")
        .map_err(|e| e.to_string())?.query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if ids.len() > 10_000 {
        return Err("音声削除履歴がバックアップの検査上限を超えています".into());
    }
    let mut meetings = std::collections::BTreeSet::new();
    for id in ids {
        if crate::storage::deletion::audio_supersession::hinted(db, &id)? {
            crate::storage::deletion::audio_supersession::inspect(
                db,
                &storage.managed_anchor,
                &id,
                check,
            )?;
            continue;
        }
        if crate::storage::deletion::audio_supersession::superseded(
            db,
            &storage.managed_anchor,
            &id,
            check,
        )? {
            continue;
        }
        if crate::storage::deletion::audio::restored::hinted(db, &id)? {
            meetings.insert(crate::storage::deletion::audio::restored::backup_meeting(
                db,
                &storage.managed_anchor,
                &id,
                check,
            )?);
            continue;
        }
        let receipt = Receipt::load(db, &id, check)?;
        verify_available_state(db, &receipt, check)?;
        if !absent(db, &storage.managed_anchor, &receipt, check)? {
            return Err(
                "削除済み音声が再出現しています。バックアップから黙って除外しません".into(),
            );
        }
        meetings.insert(receipt.saved.meeting().to_owned());
    }
    super::super::super::cancel(check)?;
    Ok(meetings)
}
pub(crate) fn finish(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    finish_inner(storage, operation, lease, check, &|_| {})
}
fn finish_inner(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    active(lease, check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let cancelled = || check() || lease.cancelled();
    let r = Receipt::load(&db, operation, &cancelled)?;
    if r.completed() {
        r.verify_anchor(storage)?;
        verify_available_state(&db, &r, &cancelled)?;
        if !absent(&db, &storage.managed_anchor, &r, &cancelled)? {
            return Err("削除済み音声が再出現しています。確認を維持します".into());
        }
        if crate::storage::deletion::startup::contains(&db, operation)? {
            let _writer = crate::storage::deletion::fence::write_audio_removal(
                storage, &db, &r, lease, &cancelled,
            )?;
            active(lease, check)?;
            r.verify_saved(&db, &cancelled)?;
            db.execute(
                "DELETE FROM temp.mon_deletion_startup_attention WHERE operation_id=?1",
                params![operation],
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(r);
    }
    if !r.files_removed() {
        return Err("音声ファイル処理が完了するまで利用不可を確定しません".into());
    }
    let _writer =
        crate::storage::deletion::fence::write_audio_removal(storage, &db, &r, lease, &cancelled)?;
    super::verify_remaining(storage, &db, &r, &cancelled)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    r.verify_saved(&tx, &cancelled)?;
    r.metadata(storage, &tx, &cancelled)?;
    tx.execute(
        "UPDATE file_assets SET state='missing' WHERE meeting_id=?1 AND media_type LIKE 'audio/%'",
        params![r.saved.meeting()],
    )
    .map_err(|e| e.to_string())?;
    event("audioAssetsUnavailable");
    tx.execute(
        "UPDATE audio_chunks SET state='missing' WHERE meeting_id=?1",
        params![r.saved.meeting()],
    )
    .map_err(|e| e.to_string())?;
    event("audioChunksUnavailable");
    tx.execute(
        "UPDATE audio_tracks SET state='failed' WHERE meeting_id=?1",
        params![r.saved.meeting()],
    )
    .map_err(|e| e.to_string())?;
    event("audioTracksUnavailable");
    let changed = r
        .saved
        .capture_scope(&tx, r.saved.meeting(), operation, &cancelled)?;
    if changed.schema_hash != r.saved.snapshot.scope.schema_hash
        || changed.maintenance_hash != r.saved.snapshot.scope.maintenance_hash
        || changed
            .tables
            .iter()
            .zip(&r.saved.snapshot.scope.tables)
            .any(|(a, b)| {
                a.rows != b.rows
                    || a.name != b.name
                    || (!matches!(
                        a.name.as_str(),
                        "file_assets" | "audio_chunks" | "audio_tracks"
                    ) && a != b)
            })
    {
        return Err("音声以外の保持履歴が確定中に変更されました".into());
    }
    let mut done = r.clone();
    done.progress.phase = Phase::AudioDeletionCompleted;
    done.progress.final_audio = Some(database::audio_tables(&tx, r.saved.meeting(), &cancelled)?);
    done.progress.validate(&done.saved, &done.targets)?;
    super::verify_tree(storage, &tx, &r, &changed, true, &cancelled)?;
    if tx.execute("UPDATE maintenance_operations SET state='completed',progress_json=?2,updated_at=max(updated_at,?3) WHERE id=?1 AND state='cleanup' AND progress_json=?4",params![operation,done.progress.encode()?,crate::storage::now_ms()?,r.progress.encode()?]).map_err(|e|e.to_string())?!=1 {return Err("音声削除完了の確定対象が変更されています".into());}
    event("beforeAudioCompletionCommit");
    done.verify_saved(&tx, &cancelled)?;
    verify_available_state(&tx, &done, &cancelled)?;
    // The journal has now transitioned to completed inside this transaction.
    // Observe with that exact receipt, not the superseded cleanup progress;
    // the typed census still verifies the saved receipt before using paths.
    super::verify_tree(storage, &tx, &done, &changed, true, &cancelled)?;
    active(lease, check)?;
    done.verify_anchor(storage)?;
    super::super::reconfirmation::clear_resolved_attention(storage, &tx, &done.saved, &cancelled)?;
    tx.commit().map_err(|e| e.to_string())?;
    // The commit wins a racing cancellation. Drop restores the normal fence
    // or the mandatory restart barrier BEFORE any post-commit callback.
    drop(_writer);
    event("audioCompletionCommitted");
    Ok(done)
}

#[cfg(test)]
#[path = "audio_completion_tests.rs"]
mod tests;
