//! A missing original is not interchangeable with an intentionally omitted cache.
use super::*;

pub(super) fn matches(db: &Connection, asset: &FileAsset, reason: &str) -> Result<bool, String> {
    let present: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='restore_omitted_assets')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !present
        || asset.state != "missing"
        || asset.size_bytes < 0
        || !portable::valid_hash(&asset.content_hash)
        || asset.relative_path != format!("meetings/{}/restored/{}", asset.meeting_id, asset.id)
    {
        return Ok(false);
    }
    let matches: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM restore_omitted_assets WHERE asset_id=?1 AND meeting_id=?2 AND reason=?3 AND source_size=?4 AND source_hash=?5) AND NOT EXISTS(SELECT 1 FROM audio_chunks WHERE file_asset_id=?1) AND NOT EXISTS(SELECT 1 FROM audio_imports WHERE original_asset_id=?1 OR converted_asset_id=?1) AND NOT EXISTS(SELECT 1 FROM audio_import_commits WHERE manifest_asset_id=?1)",params![asset.id,asset.meeting_id,reason,asset.size_bytes,asset.content_hash],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(matches)
}

pub(super) fn retain(
    db: &Connection,
    ids: &restore_ids::RestoreIds<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("復元の除外記録は公開と同時に確定してください".into());
    }
    ids.source()
        .deleted_audio
        .verify_omissions(&ids.source().manifest.omitted_assets)?;
    #[cfg(any(unix, windows))]
    let mut checked_deleted = std::collections::BTreeSet::new();
    for item in &ids.source().manifest.omitted_assets {
        check_cancelled(cancelled)?;
        let asset = ids.get("file_assets", &item.meeting_id, &item.asset_id)?;
        let meeting = ids.get("meetings", "", &item.meeting_id)?;
        let reason = match item.reason {
            archive::OmittedReason::RecreatablePcm => "recreatable_pcm",
            archive::OmittedReason::NativeLog => "native_log",
            archive::OmittedReason::SupersededAudio => {
                if !super::super::meeting_audio::superseded_asset(db, &asset)? {
                    return Err("置換済み原音声の除外証拠がありません".into());
                }
                continue;
            }
            archive::OmittedReason::SourceAudioDeleted => {
                #[cfg(not(any(unix, windows)))]
                return Err("Windowsの削除済み音声の追加復元は保留中です".into());
                #[cfg(any(unix, windows))]
                {
                    if checked_deleted.insert(item.meeting_id.clone()) {
                        let op = ids.get(
                            "operation",
                            &item.meeting_id,
                            &super::restored_audio::operation_key(
                                &ids.archive_id,
                                &item.meeting_id,
                            ),
                        )?;
                        crate::storage::deletion::audio::restored::deleted(db, &op, &meeting)?;
                    }
                    let matches: bool = db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM file_assets WHERE id=?1 AND meeting_id=?2 AND state='missing' AND media_type LIKE 'audio/%' AND relative_path=?3 AND size_bytes=?4 AND content_hash=?5)",
                        params![asset,meeting,format!("meetings/{meeting}/restored/{asset}"),i64::try_from(item.source_size).map_err(|_| "除外音声容量が範囲外です")?,item.source_sha256],|r|r.get(0),
                    ).map_err(|e|e.to_string())?;
                    if !matches {
                        return Err("音声削除の欠落資産と公開済み記録が一致しません".into());
                    }
                    // A typed persisted unavailable record, never a recreatable
                    // cache entry. Do not put original source audio in v23.
                    continue;
                }
            }
        };
        db.execute(
            "INSERT INTO restore_omitted_assets VALUES(?1,?2,?3,?4,?5)",
            params![
                asset,
                meeting,
                reason,
                i64::try_from(item.source_size).map_err(|_| "除外ファイル容量が範囲外です")?,
                item.source_sha256
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
