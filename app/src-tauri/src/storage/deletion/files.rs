//! Read-only deletion census. Serialized manifests are observations, never
//! authority to unlink. Consent, persistent fencing and removal follow.
use super::{
    database::Scope,
    fs::{Directory, Identity, Proof},
};
use crate::storage::Storage;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

pub(super) const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 32;
const MAX_MANIFEST: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Registered {
    pub id: String,
    pub size: i64,
    pub sha256: String,
    pub state: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Entry {
    pub path: String,
    pub directory: bool,
    pub proof: Option<Proof>,
    pub sha256: Option<String>,
    pub registered: Option<Registered>,
    // A known producer layout explains a leftover; it does not prove ownership.
    pub stage_hint: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Manifest {
    pub version: u32,
    pub database_scope_hash: String,
    pub meeting_id: String,
    pub root: Identity,
    pub database: Identity,
    pub entries: Vec<Entry>,
}
#[derive(Debug)]
pub(super) struct Census {
    manifest: Manifest,
    scope: Scope,
}
fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除対象ファイルの検査を取り消しました".into())
    } else {
        Ok(())
    }
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s)
}
pub(super) fn valid_path(s: &str, meeting: &str) -> bool {
    s.len() <= 256
        && s.starts_with(&format!("meetings/{meeting}/"))
        && !s.contains(['\\', '\0'])
        && s.split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".."))
}
fn hash(s: &[u8]) -> String {
    format!("{:x}", Sha256::digest(s))
}

struct Records {
    assets: BTreeMap<String, Registered>,
    sessions: BTreeSet<String>,
    imports: BTreeSet<String>,
    jobs: BTreeMap<String, String>,
}
impl Records {
    fn read(db: &Connection, meeting: &str) -> Result<Self, String> {
        let mut q=db.prepare("SELECT id,relative_path,size_bytes,content_hash,state FROM file_assets WHERE meeting_id=?1 ORDER BY relative_path").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![meeting], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    Registered {
                        id: r.get(0)?,
                        size: r.get(2)?,
                        sha256: r.get(3)?,
                        state: r.get(4)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut assets = BTreeMap::new();
        for row in rows {
            let (path, asset) = row.map_err(|e| e.to_string())?;
            if assets.len() >= MAX_ENTRIES
                || !valid_path(&path, meeting)
                || asset.size < 0
                || asset.sha256.len() != 64
                || !asset
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("登録資産の管理パス・SHA・件数が不正です".into());
            }
            if assets.insert(path, asset).is_some() {
                return Err("登録資産のパスが重複しています".into());
            }
        }
        let ids = |table: &str| -> Result<BTreeSet<String>, String> {
            db.prepare(&format!(
                "SELECT id FROM {table} WHERE meeting_id=?1 ORDER BY id"
            ))
            .map_err(|e| e.to_string())?
            .query_map(params![meeting], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
        };
        let jobs = db
            .prepare("SELECT id,kind FROM processing_jobs WHERE meeting_id=?1 ORDER BY id")
            .map_err(|e| e.to_string())?
            .query_map(params![meeting], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            assets,
            sessions: ids("recording_sessions")?,
            imports: ids("audio_imports")?,
            jobs,
        })
    }
    fn hint(&self, path: &str) -> Option<String> {
        let p: Vec<_> = path.split('/').collect();
        if p.len() == 5 && uuid(p[3]) {
            if p[2] == "audio"
                && self.sessions.contains(p[3])
                && matches!(p[4], "microphone.caf" | "system.caf" | "manifest.json")
            {
                return Some("recording".into());
            }
            if p[2] == "imports"
                && self.imports.contains(p[3])
                && matches!(
                    p[4],
                    "original.wav"
                        | "original.mp3"
                        | "original.m4a"
                        | "original.flac"
                        | "audio.flac"
                        | "decoded.caf"
                        | "manifest.json"
                )
            {
                return Some("import".into());
            }
        }
        if p.len() == 6 && p[2] == "analysis" && uuid(p[3]) && unit(p[4]) {
            let kind = self.jobs.get(p[3])?;
            let allowed = match kind.as_str() {
                "transcription" => matches!(
                    p[5],
                    "input.wav"
                        | "result.json"
                        | "logical-result.json"
                        | "stdout.log"
                        | "stderr.log"
                ),
                "diarization" => matches!(
                    p[5],
                    "input.wav"
                        | "result.json"
                        | "stdout.log"
                        | "logical-stdout.log"
                        | "stderr.log"
                ),
                "minutes_generation" => matches!(
                    p[5],
                    "system.txt"
                        | "prompt.txt"
                        | "schema.json"
                        | "result.json"
                        | "items.json"
                        | "stderr.log"
                ),
                _ => false,
            };
            if allowed {
                return Some(kind.clone());
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Summary {
    pub files: usize,
    pub directories: usize,
    pub missing: usize,
    pub changed: usize,
    pub unregistered_with_hint: usize,
    pub unknown: usize,
    pub bytes: u64,
}
fn unit(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("unit-") else {
        return false;
    };
    let Some((index, id)) = rest.split_once('-') else {
        return false;
    };
    index.parse::<usize>().is_ok_and(|n| n.to_string() == index) && uuid(id)
}

fn walk(
    dir: &Directory,
    path: &str,
    depth: usize,
    entries: &mut BTreeMap<String, Entry>,
    records: &Records,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    cancel(check)?;
    if depth > MAX_DEPTH || entries.len() >= MAX_ENTRIES || path.len() > 256 {
        return Err("削除対象の深さ・パス・件数が安全上限を超えています".into());
    }
    let before = dir.proof()?;
    entries.insert(
        path.into(),
        Entry {
            path: path.into(),
            directory: true,
            proof: Some(before.clone()),
            sha256: None,
            registered: None,
            stage_hint: None,
        },
    );
    let names = dir.names(MAX_ENTRIES - entries.len())?;
    for name in &names {
        cancel(check)?;
        let child_path = format!("{path}/{name}");
        if child_path.len() > 256 || entries.len() >= MAX_ENTRIES {
            return Err("削除対象のパス・件数が安全上限を超えています".into());
        }
        if dir.kind(name)? {
            let child = dir
                .child(name)?
                .ok_or("検査中に対象フォルダーが消失しました")?;
            walk(&child, &child_path, depth + 1, entries, records, check)?;
        } else {
            let mut file = dir.regular(name)?;
            let before = Proof::read(&file.metadata().map_err(|e| e.to_string())?)?;
            let mut digest = Sha256::new();
            let mut length = 0u64;
            // Keep the buffer off recursive directory-walk stack frames.
            let mut buffer = vec![0u8; 128 * 1024];
            loop {
                cancel(check)?;
                let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                length = length
                    .checked_add(n as u64)
                    .ok_or("ファイル容量が範囲外です")?;
                if length > before.size {
                    return Err("検査中にファイルが拡張されました".into());
                }
                digest.update(&buffer[..n]);
            }
            dir.verify_file(name, &file)?;
            if length != before.size
                || Proof::read(&file.metadata().map_err(|e| e.to_string())?)? != before
            {
                return Err("検査中にファイルが変更されました".into());
            }
            entries.insert(
                child_path.clone(),
                Entry {
                    path: child_path.clone(),
                    directory: false,
                    proof: Some(before),
                    sha256: Some(format!("{:x}", digest.finalize())),
                    registered: records.assets.get(&child_path).cloned(),
                    stage_hint: records.hint(&child_path),
                },
            );
        }
    }
    if dir.proof()? != before || dir.names(MAX_ENTRIES)? != names {
        return Err("検査中にフォルダーの内容が変更されました".into());
    }
    Ok(())
}
fn inventory(
    root: &Directory,
    meeting: &str,
    records: &Records,
    check: &impl Fn() -> bool,
) -> Result<Vec<Entry>, String> {
    root.verify()?;
    let mut entries = BTreeMap::new();
    if let Some(meetings) = root.child("meetings")? {
        if let Some(directory) = meetings.child(meeting)? {
            walk(
                &directory,
                &format!("meetings/{meeting}"),
                0,
                &mut entries,
                records,
                check,
            )?;
        }
    }
    for (path, asset) in &records.assets {
        if !entries.contains_key(path) {
            if entries.len() >= MAX_ENTRIES {
                return Err("削除対象の件数が安全上限を超えています".into());
            }
            entries.insert(
                path.clone(),
                Entry {
                    path: path.clone(),
                    directory: false,
                    proof: None,
                    sha256: None,
                    registered: Some(asset.clone()),
                    stage_hint: None,
                },
            );
        } else if entries[path].directory {
            return Err("登録ファイルのパスがフォルダーへ変更されています".into());
        }
    }
    root.verify()?;
    Ok(entries.into_values().collect())
}

impl Census {
    pub(super) fn capture(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancel(check)?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        Self::capture_locked(storage, &db, meeting, "", check)
    }
    // The durable-intent transaction already holds the connection mutex.
    pub(super) fn capture_locked(
        storage: &Storage,
        db: &Connection,
        meeting: &str,
        omitted_operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        if storage.restart_required() {
            return Err("旧保存先は停止済みです。再起動してください".into());
        }
        if storage
            .maintenance
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("保存領域の操作または書込停止が既に進行中です".into());
        }
        let (root, database) = storage.managed_anchor.identity()?;
        let scope = Scope::capture(db, meeting, omitted_operation, check)?;
        let records = Records::read(db, meeting)?;
        // Enumerate through the startup descriptor, never reopen a path and
        // temporarily treat its replacement as the original root.
        let root_directory = storage.managed_anchor.directory_ref()?;
        let entries = inventory(root_directory, meeting, &records, check)?;
        if inventory(root_directory, meeting, &records, check)? != entries
            || Scope::capture(db, meeting, omitted_operation, check)? != scope
            || storage.managed_anchor.identity()? != (root.clone(), database.clone())
        {
            return Err("確認中に削除対象が変更されました".into());
        }
        let result = Self {
            manifest: Manifest {
                version: 1,
                database_scope_hash: scope.hash()?,
                meeting_id: meeting.into(),
                root,
                database,
                entries,
            },
            scope,
        };
        result.encode()?;
        Ok(result)
    }
    pub(super) fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub(super) fn scope(&self) -> &Scope {
        &self.scope
    }
    pub(super) fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            Err("ファイルmanifestが安全上限を超えています".into())
        } else {
            Ok(bytes)
        }
    }
    pub(super) fn hash(&self) -> Result<String, String> {
        Ok(hash(&self.encode()?))
    }
    pub(super) fn summary(&self) -> Result<Summary, String> {
        let mut result = Summary {
            files: 0,
            directories: 0,
            missing: 0,
            changed: 0,
            unregistered_with_hint: 0,
            unknown: 0,
            bytes: 0,
        };
        for entry in &self.manifest.entries {
            if entry.directory {
                result.directories += 1;
                continue;
            }
            result.files += 1;
            if let Some(proof) = &entry.proof {
                result.bytes = result
                    .bytes
                    .checked_add(proof.size)
                    .ok_or("削除対象容量が範囲外です")?;
                if let Some(registered) = &entry.registered {
                    if u64::try_from(registered.size).ok() != Some(proof.size)
                        || entry.sha256.as_deref() != Some(registered.sha256.as_str())
                    {
                        result.changed += 1;
                    }
                } else if entry.stage_hint.is_some() {
                    result.unregistered_with_hint += 1;
                } else {
                    result.unknown += 1;
                }
            } else {
                result.missing += 1;
            }
        }
        Ok(result)
    }
    pub(super) fn verify(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let current = Self::capture(storage, &self.scope.meeting_id, check)?;
        if current.scope != self.scope || current.manifest != self.manifest {
            return Err("確認後にDB・ファイル・実体が変更されています。再確認してください".into());
        }
        Ok(())
    }
}
