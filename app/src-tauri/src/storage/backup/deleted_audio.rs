//! Portable absence is a historical fact, not permission to remove a file.
//! Only typed native completion and anchored absence may create this fact on
//! export. Imported descriptors must match the owned logical DB and omissions.
use super::*;
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: u64 = 200_000;
pub(super) const MAX_MEETINGS: usize = 10_000;
const TABLES: [(&str, &str); 3] = [
    ("audio_chunks", "meeting_id=?1"),
    ("audio_tracks", "meeting_id=?1"),
    ("file_assets", "meeting_id=?1 AND media_type LIKE 'audio/%'"),
];

pub(super) struct Native {
    meetings: BTreeSet<String>,
}
impl Native {
    pub(super) fn empty() -> Self {
        Self {
            meetings: BTreeSet::new(),
        }
    }
    pub(super) fn capture(
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        #[cfg(unix)]
        let meetings = crate::storage::deletion::audio::backup_deleted(storage, db, check)?;
        #[cfg(not(unix))]
        let meetings = {
            let _ = (storage, db);
            check_cancelled(check)?;
            BTreeSet::new()
        };
        Ok(Self { meetings })
    }
    pub(super) fn verify(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if Self::capture(storage, db, check)?.meetings != self.meetings {
            return Err("バックアップの音声削除履歴が変更されています".into());
        }
        Ok(())
    }
    pub(super) fn contains_asset(&self, asset: &FileAsset) -> bool {
        self.meetings.contains(&asset.meeting_id)
            && audio_media(&asset.media_type)
            && asset.state == "missing"
    }
    pub(super) fn is_empty(&self) -> bool {
        self.meetings.is_empty()
    }

    // Capture AFTER path/config redaction. Native row hashes bind machine paths,
    // device fields and IDs and must never be reused as portable fingerprints.
    pub(super) fn bind(
        &self,
        logical: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Vec<Descriptor>, String> {
        self.meetings
            .iter()
            .map(|meeting| {
                check_cancelled(check)?;
                unavailable(logical, meeting)?;
                Ok(Descriptor {
                    version: 1,
                    meeting_id: meeting.clone(),
                    reason: Reason::SourceAudioDeleted,
                    tables: fingerprints(logical, meeting, check)?,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    SourceAudioDeleted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TableProof {
    name: String,
    rows: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Descriptor {
    version: u32,
    pub(super) meeting_id: String,
    reason: Reason,
    tables: Vec<TableProof>,
}

pub(super) fn audio_media(media: &str) -> bool {
    media
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("audio/"))
}
fn uuid(s: &str) -> bool {
    Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}

impl Descriptor {
    pub(super) fn validate_shape(&self) -> Result<(), String> {
        if self.version != 1
            || !uuid(&self.meeting_id)
            || self.tables.len() != TABLES.len()
            || self.tables.iter().zip(TABLES).any(|(t, (name, _))| {
                t.name != name || t.rows > MAX_ROWS || !portable::valid_hash(&t.sha256)
            })
        {
            return Err("音声削除の持ち運び用履歴指紋が不正です".into());
        }
        Ok(())
    }
}

// Private validated view for later historical reconstruction. No filesystem
// paths or writable native operation can be obtained from this capability.
pub(in crate::storage) struct Verified {
    // Keep the validated proofs independently of the mutable manifest. A
    // previously checked meeting ID alone cannot authorize historical reads
    // after the logical database has changed.
    proofs: BTreeMap<String, Vec<TableProof>>,
    assets: BTreeMap<String, MissingSourceAsset>,
}
pub(super) struct MissingSourceAsset {
    meeting: String,
    pub(super) media: String,
    pub(super) size: i64,
    pub(super) hash: String,
}
impl Verified {
    pub(super) fn meetings(&self) -> impl Iterator<Item = &str> {
        self.proofs.keys().map(String::as_str)
    }

    pub(super) fn verify_omissions(&self, omitted: &[archive::OmittedAsset]) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for item in omitted {
            if let Some(asset) = self.asset(&item.meeting_id, &item.asset_id) {
                if item.reason != archive::OmittedReason::SourceAudioDeleted
                    || item.source_size != asset.size as u64
                    || item.source_sha256 != asset.hash
                    || !seen.insert(item.asset_id.clone())
                {
                    return Err("検査済み音声削除履歴と除外理由・由来が一致しません".into());
                }
            } else if item.reason == archive::OmittedReason::SourceAudioDeleted
                || (item.reason == archive::OmittedReason::RecreatablePcm
                    && self.contains(&item.meeting_id))
            {
                return Err("音声削除履歴のない欠落・再生成可能PCMは採用しません".into());
            }
        }
        if seen.len() != self.assets.len() {
            return Err("検査済み音声削除履歴の全資産が除外記録にありません".into());
        }
        Ok(())
    }

    pub(super) fn asset(&self, meeting: &str, id: &str) -> Option<&MissingSourceAsset> {
        self.assets.get(id).filter(|a| a.meeting == meeting)
    }
    pub(super) fn contains(&self, meeting: &str) -> bool {
        self.proofs.contains_key(meeting)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.proofs.is_empty()
    }
    pub(in crate::storage) fn verify_meeting(
        &self,
        db: &Connection,
        meeting: &str,
    ) -> Result<(), String> {
        let expected = self
            .proofs
            .get(meeting)
            .ok_or("検査済みの音声削除履歴がありません")?;
        unavailable(db, meeting)?;
        if &fingerprints(db, meeting, &|| false)? != expected {
            return Err("固定入力の音声削除履歴と現在のDBが一致しません".into());
        }
        Ok(())
    }
}

pub(super) fn verify(
    db: &Connection,
    descriptors: &[Descriptor],
    omitted: &[archive::OmittedAsset],
    check: &impl Fn() -> bool,
) -> Result<Verified, String> {
    if descriptors.len() > MAX_MEETINGS {
        return Err("音声削除の持ち運び用会議件数が検査上限を超えています".into());
    }
    let mut meetings = BTreeSet::new();
    let mut expected = BTreeMap::new();
    let mut assets = BTreeMap::new();
    for item in descriptors {
        check_cancelled(check)?;
        item.validate_shape()?;
        if !meetings.insert(item.meeting_id.clone()) {
            return Err("音声削除の会議が重複しています".into());
        }
        unavailable(db, &item.meeting_id)?;
        if fingerprints(db, &item.meeting_id, check)? != item.tables {
            return Err("音声削除の持ち運び用履歴とDBが一致しません".into());
        }
        let mut statement = db.prepare("SELECT id,media_type,size_bytes,content_hash FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' ORDER BY id").map_err(|e| e.to_string())?;
        let ids = statement
            .query_map([&item.meeting_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for id in ids {
            check_cancelled(check)?;
            let (id, media, size, hash) = id.map_err(|e| e.to_string())?;
            if size < 0 || !portable::valid_hash(&hash) {
                return Err("削除済み音声の容量・ハッシュが不正です".into());
            }
            expected.insert(id.clone(), item.meeting_id.clone());
            assets.insert(
                id,
                MissingSourceAsset {
                    meeting: item.meeting_id.clone(),
                    media,
                    size,
                    hash,
                },
            );
        }
    }
    let mut actual = BTreeMap::new();
    for item in omitted {
        check_cancelled(check)?;
        if item.reason == archive::OmittedReason::SourceAudioDeleted {
            if actual
                .insert(item.asset_id.clone(), item.meeting_id.clone())
                .is_some()
            {
                return Err("削除済み音声の除外記録が重複しています".into());
            }
        } else if item.reason == archive::OmittedReason::RecreatablePcm
            && meetings.contains(&item.meeting_id)
        {
            return Err("元音声の削除後にPCMを再生成可能として扱いません".into());
        }
    }
    if expected != actual {
        return Err("削除済み音声の全資産と除外理由が一致しません".into());
    }
    check_cancelled(check)?;
    Ok(Verified {
        assets,
        proofs: descriptors
            .iter()
            .map(|d| (d.meeting_id.clone(), d.tables.clone()))
            .collect(),
    })
}

fn unavailable(db: &Connection, meeting: &str) -> Result<(), String> {
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state!='missing') AND NOT EXISTS(SELECT 1 FROM audio_chunks WHERE meeting_id=?1 AND state!='missing') AND NOT EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND state!='failed')", [meeting], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !uuid(meeting) || !valid {
        return Err("削除済み音声の会議・利用不可状態が一致しません".into());
    }
    Ok(())
}

fn fingerprints(
    db: &Connection,
    meeting: &str,
    check: &impl Fn() -> bool,
) -> Result<Vec<TableProof>, String> {
    TABLES
        .iter()
        .map(|(name, predicate)| {
            check_cancelled(check)?;
            // Identifiers/predicates are repository-owned, never archive-supplied.
            let mut statement = db
                .prepare(&format!(
                    "SELECT * FROM {name} WHERE {predicate} ORDER BY id"
                ))
                .map_err(|e| e.to_string())?;
            let columns = statement.column_count();
            let mut hash = Sha256::new();
            hash.update(b"mon/portable-deleted-audio/v1\0");
            hash.update(name.as_bytes());
            hash.update(serde_json::to_vec(&statement.column_names()).map_err(|e| e.to_string())?);
            let mut rows = statement.query([meeting]).map_err(|e| e.to_string())?;
            let mut count = 0;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(check)?;
                count += 1;
                if count > MAX_ROWS {
                    return Err("持ち運び用音声履歴の件数が検査上限を超えています".into());
                }
                hash.update([0xff]);
                let mut bytes = 0usize;
                for i in 0..columns {
                    match row.get_ref(i).map_err(|e| e.to_string())? {
                        ValueRef::Null => hash.update([0]),
                        ValueRef::Integer(n) => {
                            hash.update([1]);
                            hash.update(n.to_le_bytes());
                        }
                        ValueRef::Real(n) if n.is_finite() => {
                            hash.update([2]);
                            hash.update(n.to_bits().to_le_bytes());
                        }
                        ValueRef::Text(s) if std::str::from_utf8(s).is_ok() => {
                            bytes = bytes
                                .checked_add(s.len())
                                .ok_or("音声履歴の行容量が範囲外です")?;
                            if bytes > portable::MAX_ROW_BYTES {
                                return Err(
                                    "持ち運び用音声履歴の行容量が検査上限を超えています".into()
                                );
                            }
                            hash.update([3]);
                            hash.update((s.len() as u64).to_le_bytes());
                            hash.update(s);
                        }
                        _ => return Err("持ち運び用音声履歴に未対応の値があります".into()),
                    }
                }
            }
            Ok(TableProof {
                name: name.to_string(),
                rows: count,
                sha256: format!("{:x}", hash.finalize()),
            })
        })
        .collect()
}
