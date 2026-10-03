use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

mod analysis;
mod drafts;
mod jobs;
mod playback;
mod recordings;
mod revisions;
pub use analysis::AnalysisView;
pub use drafts::{TranscriptDraft, TranscriptDraftPayload};
pub use recordings::RecordingConfig;

const MIGRATION_1: &str = include_str!("migrations/0001_meetings.sql");
const MIGRATION_2: &str = include_str!("migrations/0002_file_assets.sql");
const MIGRATION_3: &str = include_str!("migrations/0003_revisions.sql");
const MIGRATION_4: &str = include_str!("migrations/0004_audio.sql");
const MIGRATION_5: &str = include_str!("migrations/0005_content.sql");
const MIGRATION_6: &str = include_str!("migrations/0006_drafts.sql");
const MIGRATION_7: &str = include_str!("migrations/0007_jobs.sql");
const MIGRATION_8: &str = include_str!("migrations/0008_recording_guards.sql");

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub lifecycle: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub lock_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAsset {
    pub id: String,
    pub meeting_id: String,
    pub relative_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub content_hash: String,
    pub state: String,
}

pub struct Storage {
    root: PathBuf,
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(root: PathBuf) -> Result<Self, Box<dyn Error>> {
        std::fs::create_dir_all(&root)?;
        let mut connection = Connection::open(root.join("meetings.sqlite3"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA main.locking_mode = EXCLUSIVE; BEGIN IMMEDIATE; COMMIT;",
        )?;
        apply_migrations(&mut connection)?;
        connection.execute(
            "UPDATE processing_jobs SET state = 'interrupted', updated_at = ?1 WHERE state IN ('running', 'pausing')",
            params![now_ms()?],
        )?;
        connection.execute(
            "UPDATE recording_sessions SET state = 'interrupted', updated_at = ?1 WHERE state IN ('preparing', 'recording', 'stopping')",
            params![now_ms()?],
        )?;
        Ok(Self {
            root,
            connection: Mutex::new(connection),
        })
    }

    pub fn create_meeting(&self, title: &str) -> Result<Meeting, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("会議名は1～200文字で入力してください".into());
        }
        let now = now_ms()?;
        let meeting = Meeting {
            id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            lifecycle: "draft".into(),
            created_at: now,
            updated_at: now,
            lock_version: 0,
        };
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .execute(
                "INSERT INTO meetings (id, title, lifecycle, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![meeting.id, meeting.title, meeting.lifecycle, now, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(meeting)
    }

    pub fn list_meetings(&self) -> Result<Vec<Meeting>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let mut statement = connection
            .prepare("SELECT id, title, lifecycle, created_at, updated_at, lock_version FROM meetings ORDER BY updated_at DESC, id DESC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(Meeting {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    lifecycle: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    lock_version: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_meeting(&self, id: &str) -> Result<Option<Meeting>, String> {
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .query_row(
                "SELECT id, title, lifecycle, created_at, updated_at, lock_version FROM meetings WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Meeting {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        lifecycle: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        lock_version: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn rename_meeting(
        &self,
        id: &str,
        title: &str,
        expected_lock_version: i64,
    ) -> Result<Meeting, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("会議名は1～200文字で入力してください".into());
        }
        if expected_lock_version < 0 {
            return Err("会議の版番号が不正です".into());
        }
        let now = now_ms()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let changed = connection
            .execute(
                "UPDATE meetings SET title = ?1, updated_at = ?2, lock_version = lock_version + 1 WHERE id = ?3 AND lock_version = ?4",
                params![title, now, id, expected_lock_version],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            return Err(if exists {
                "別の画面で会議が更新されました。再読込してください"
            } else {
                "会議が見つかりません"
            }
            .into());
        }
        connection
            .query_row(
                "SELECT id, title, lifecycle, created_at, updated_at, lock_version FROM meetings WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Meeting {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        lifecycle: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        lock_version: row.get(5)?,
                    })
                },
            )
            .map_err(|error| error.to_string())
    }

    // Only identifiers generated by the app may form managed paths. Never use a meeting title.
    #[allow(dead_code)]
    pub fn managed_path(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("管理領域から外れるパスです".into());
        }
        Ok(self.root.join(relative))
    }

    // Called only after the recorder has finished and closed a managed file.
    // The UI cannot provide an arbitrary source path to this method.
    #[allow(dead_code)]
    pub fn register_ready_meeting_asset(
        &self,
        meeting_id: &str,
        relative: &Path,
        media_type: &str,
    ) -> Result<FileAsset, String> {
        let mut parts = relative.components();
        if !matches!(parts.next(), Some(Component::Normal(name)) if name == "meetings")
            || !matches!(parts.next(), Some(Component::Normal(name)) if name == meeting_id)
            || parts.next().is_none()
        {
            return Err("会議の管理パスが不正です".into());
        }
        if media_type.is_empty() || media_type.len() > 100 {
            return Err("メディア形式が不正です".into());
        }
        let relative_text = relative
            .to_str()
            .ok_or("管理パスを文字列化できません")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative_text.contains('\\') {
            return Err("管理パスに使用できない文字があります".into());
        }
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if !actual.starts_with(&root) || actual != root.join(relative) {
            return Err("管理領域から外れるファイルです".into());
        }
        if !actual.is_file() {
            return Err("管理対象は通常ファイルに限ります".into());
        }
        let (size_bytes, content_hash) = hash_file(&actual)?;
        let now = now_ms()?;
        let asset = FileAsset {
            id: Uuid::new_v4().to_string(),
            meeting_id: meeting_id.to_owned(),
            relative_path: relative_text,
            media_type: media_type.to_owned(),
            size_bytes,
            content_hash,
            state: "ready".into(),
        };
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .execute(
                "INSERT INTO file_assets (id, meeting_id, storage_scope, relative_path, media_type, size_bytes, hash_algorithm, content_hash, state, created_at, updated_at) VALUES (?1, ?2, 'meeting', ?3, ?4, ?5, 'sha256', ?6, 'ready', ?7, ?7)",
                params![asset.id, asset.meeting_id, asset.relative_path, asset.media_type, asset.size_bytes, asset.content_hash, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(asset)
    }

    #[allow(dead_code)]
    pub fn verify_asset(&self, asset_id: &str) -> Result<bool, String> {
        let asset: Option<FileAsset> = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?
            .query_row(
                "SELECT id, meeting_id, relative_path, media_type, size_bytes, content_hash, state FROM file_assets WHERE id = ?1",
                params![asset_id],
                |row| Ok(FileAsset {
                    id: row.get(0)?, meeting_id: row.get(1)?, relative_path: row.get(2)?, media_type: row.get(3)?,
                    size_bytes: row.get(4)?, content_hash: row.get(5)?, state: row.get(6)?,
                }),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let asset = asset.ok_or("管理ファイルが見つかりません")?;
        let relative = Path::new(&asset.relative_path);
        let file = self.managed_path(relative)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let actual = file.canonicalize().map_err(|error| error.to_string())?;
        if !actual.starts_with(&root) || actual != root.join(relative) {
            return Err("管理領域から外れるファイルです".into());
        }
        let (size_bytes, content_hash) = hash_file(&actual)?;
        Ok(size_bytes == asset.size_bytes && content_hash == asset.content_hash)
    }
}

fn apply_migrations(connection: &mut Connection) -> Result<(), Box<dyn Error>> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);",
    )?;
    let known: Vec<(i64, String)> = {
        let mut query = connection
            .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let migrations = [
        (1_i64, MIGRATION_1),
        (2_i64, MIGRATION_2),
        (3_i64, MIGRATION_3),
        (4_i64, MIGRATION_4),
        (5_i64, MIGRATION_5),
        (6_i64, MIGRATION_6),
        (7_i64, MIGRATION_7),
        (8_i64, MIGRATION_8),
    ];
    for (index, (version, checksum)) in known.iter().enumerate() {
        let Some((expected_version, sql)) = migrations.get(index) else {
            return Err(format!("unsupported migration {version}").into());
        };
        let digest = format!("{:x}", Sha256::digest(sql.as_bytes()));
        if version != expected_version || checksum != &digest {
            return Err(format!("unsupported or changed migration {version}").into());
        }
    }
    for (version, sql) in migrations.into_iter().skip(known.len()) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        let digest = format!("{:x}", Sha256::digest(sql.as_bytes()));
        transaction.execute(
            "INSERT INTO schema_migrations (version, checksum, applied_at) VALUES (?1, ?2, ?3)",
            params![version, digest, now_ms()?],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(i64, String), String> {
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut size = 0_i64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(i64::try_from(count).map_err(|error| error.to_string())?)
            .ok_or("ファイルが大きすぎます")?;
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn now_ms() -> Result<i64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let created = storage.create_meeting("  定例会  ").unwrap();
        assert_eq!(created.title, "定例会");
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(reopened.list_meetings().unwrap(), vec![created]);
    }

    #[test]
    fn second_instance_cannot_recover_live_recording_as_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let first = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = first.create_meeting("録音中").unwrap();
        let session = first
            .prepare_recording(
                &meeting.id,
                &RecordingConfig {
                    mode: "system".into(),
                    backend: "screencapturekit".into(),
                    target_pids: vec![],
                },
                "start",
            )
            .unwrap();
        first.mark_recording_started(&session.id).unwrap();
        assert!(Storage::open(temp.path().to_owned()).is_err());
        let state: String = first
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM recording_sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "recording");
        drop(first);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        let recovered: String = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM recording_sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered, "interrupted");
    }

    #[test]
    fn path_cannot_escape_root() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.managed_path(Path::new("../outside")).is_err());
        assert!(storage.managed_path(Path::new("/tmp/outside")).is_err());
        assert!(storage
            .managed_path(Path::new("meetings/id/audio/file.flac"))
            .is_ok());
    }

    #[test]
    fn changed_migration_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE schema_migrations SET checksum = 'changed' WHERE version = 1",
                [],
            )
            .unwrap();
        drop(storage);
        assert!(Storage::open(temp.path().to_owned()).is_err());
    }

    #[test]
    fn empty_title_is_rejected_without_insert() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.create_meeting("  ").is_err());
        assert!(storage.list_meetings().unwrap().is_empty());
    }

    #[test]
    fn rename_requires_current_version_and_survives_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let created = storage.create_meeting("旧名称").unwrap();
        let renamed = storage
            .rename_meeting(&created.id, " 新名称 ", created.lock_version)
            .unwrap();
        assert_eq!(renamed.title, "新名称");
        assert_eq!(renamed.lock_version, created.lock_version + 1);
        assert!(storage
            .rename_meeting(&created.id, "古い更新", created.lock_version)
            .is_err());
        drop(storage);
        let reopened = Storage::open(temp.path().to_owned()).unwrap();
        assert_eq!(reopened.get_meeting(&created.id).unwrap(), Some(renamed));
    }

    #[test]
    fn missing_meeting_cannot_be_renamed() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        assert!(storage.rename_meeting("missing", "name", 0).is_err());
    }

    #[test]
    fn older_database_is_migrated_without_losing_meetings() {
        let temp = tempfile::tempdir().unwrap();
        let mut connection = Connection::open(temp.path().join("meetings.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(MIGRATION_1).unwrap();
        let digest = format!("{:x}", Sha256::digest(MIGRATION_1.as_bytes()));
        transaction
            .execute(
                "INSERT INTO schema_migrations VALUES (1, ?1, 0)",
                params![digest],
            )
            .unwrap();
        transaction.commit().unwrap();
        drop(connection);

        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let created = storage.create_meeting("移行後の会議").unwrap();
        let count: i64 = storage
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 8);
        assert_eq!(storage.get_meeting(&created.id).unwrap(), Some(created));
    }

    #[test]
    fn managed_asset_is_hashed_and_tampering_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("音声").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", meeting.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio bytes").unwrap();
        let asset = storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .unwrap();
        assert_eq!(asset.size_bytes, 11);
        assert!(storage.verify_asset(&asset.id).unwrap());
        assert!(storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .is_err());
        std::fs::write(&file, b"audio bytes changed").unwrap();
        assert!(!storage.verify_asset(&asset.id).unwrap());
    }

    #[test]
    fn asset_cannot_cross_meeting_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", one.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio").unwrap();
        assert!(storage
            .register_ready_meeting_asset(&two.id, &relative, "audio/flac")
            .is_err());
        assert!(storage
            .register_ready_meeting_asset("missing", &relative, "audio/flac")
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_outside_is_not_registered() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("音声").unwrap();
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", meeting.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let outside_file = outside.path().join("other.flac");
        std::fs::write(&outside_file, b"private").unwrap();
        symlink(&outside_file, &file).unwrap();
        assert!(storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .is_err());
    }

    #[test]
    fn revision_heads_require_committed_revisions_from_same_meeting() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let db = storage.connection.lock().unwrap();
        let analysis_id = Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, 1, 'engine', 'candidate', ?3, '{}', '{}', ?4, 1, 0)",
            params![analysis_id, one.id, "a".repeat(64), "b".repeat(64)],
        ).unwrap();
        assert!(db
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
                params![analysis_id, one.id]
            )
            .is_err());
        db.execute(
            "UPDATE analysis_revisions SET publication_state = 'committed' WHERE id = ?1",
            params![analysis_id],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
                params![analysis_id, two.id]
            )
            .is_err());
        db.execute(
            "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
            params![analysis_id, one.id],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE analysis_revisions SET content_hash = ?1 WHERE id = ?2",
                params!["c".repeat(64), analysis_id]
            )
            .is_err());
        assert!(db
            .execute(
                "DELETE FROM analysis_revisions WHERE id = ?1",
                params![analysis_id]
            )
            .is_err());
        let child_id = Uuid::new_v4().to_string();
        assert!(db.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, parent_revision_id, revision_no, parent_content_hash, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, 2, ?4, 'engine', 'candidate', ?5, '{}', '{}', ?6, 1, 0)",
            params![child_id, one.id, analysis_id, "wrong".to_owned(), "a".repeat(64), "d".repeat(64)],
        ).is_err());
    }

    #[test]
    fn minutes_input_hash_and_head_are_checked() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("minutes").unwrap();
        let db = storage.connection.lock().unwrap();
        let analysis_id = Uuid::new_v4().to_string();
        let minutes_id = Uuid::new_v4().to_string();
        let content_hash = "b".repeat(64);
        db.execute(
            "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, 1, 'engine', 'candidate', ?3, '{}', '{}', ?4, 1, 0)",
            params![analysis_id, meeting.id, "a".repeat(64), content_hash],
        ).unwrap();
        let insert_minutes = "INSERT INTO minutes_revisions (id, meeting_id, input_analysis_id, revision_no, publication_state, generated_document_json, effective_document_json, document_schema_version, input_content_hash, content_hash, model_snapshot_json, template_version, created_at) VALUES (?1, ?2, ?3, 1, 'candidate', '{}', '{}', 1, ?4, ?5, '{}', 'v1', 0)";
        assert!(db
            .execute(
                insert_minutes,
                params![
                    minutes_id,
                    meeting.id,
                    analysis_id,
                    content_hash,
                    "d".repeat(64)
                ]
            )
            .is_err());
        db.execute(
            "UPDATE analysis_revisions SET publication_state = 'committed' WHERE id = ?1",
            params![analysis_id],
        )
        .unwrap();
        assert!(db
            .execute(
                insert_minutes,
                params![
                    minutes_id,
                    meeting.id,
                    analysis_id,
                    "c".repeat(64),
                    "d".repeat(64)
                ]
            )
            .is_err());
        db.execute(
            insert_minutes,
            params![
                minutes_id,
                meeting.id,
                analysis_id,
                content_hash,
                "d".repeat(64)
            ],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE meetings SET current_minutes_id = ?1 WHERE id = ?2",
                params![minutes_id, meeting.id]
            )
            .is_err());
        db.execute(
            "UPDATE minutes_revisions SET publication_state = 'committed' WHERE id = ?1",
            params![minutes_id],
        )
        .unwrap();
        db.execute(
            "UPDATE meetings SET current_minutes_id = ?1 WHERE id = ?2",
            params![minutes_id, meeting.id],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE minutes_revisions SET effective_document_json = '{}' WHERE id = ?1",
                params![minutes_id]
            )
            .is_err());
    }

    #[test]
    fn recording_tracks_and_chunks_cannot_cross_meetings() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let one = storage.create_meeting("one").unwrap();
        let two = storage.create_meeting("two").unwrap();
        let session_id = Uuid::new_v4().to_string();
        let track_id = Uuid::new_v4().to_string();
        let insert_session = "INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES (?1, ?2, 'preparing', 'system', '{}', ?3, 0, 0)";
        {
            let db = storage.connection.lock().unwrap();
            assert!(db
                .execute(insert_session, params![session_id, one.id, "start-one"])
                .is_err());
            db.execute(
                "UPDATE meetings SET source_type = 'recording' WHERE id IN (?1, ?2)",
                params![one.id, two.id],
            )
            .unwrap();
            db.execute(insert_session, params![session_id, one.id, "start-one"])
                .unwrap();
            let insert_track = "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES (?1, ?2, ?3, 'mic', 'flac', 48000, 1, 'ready', 0, 0)";
            assert!(db
                .execute(insert_track, params![track_id, two.id, session_id])
                .is_err());
            db.execute(insert_track, params![track_id, one.id, session_id])
                .unwrap();
            db.execute(
                "INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('derived-track', ?1, ?2, 'processed', 'flac', 48000, 1, 'ready', 0, 0)",
                params![one.id, session_id],
            ).unwrap();
            db.execute("INSERT INTO track_inputs (meeting_id, derived_track_id, input_track_id, role) VALUES (?1, 'derived-track', ?2, 'source')", params![one.id, track_id]).unwrap();
            assert!(db.execute("INSERT INTO track_inputs (meeting_id, derived_track_id, input_track_id, role) VALUES (?1, ?2, 'derived-track', 'source')", params![one.id, track_id]).is_err());
            assert!(db
                .execute(
                    "UPDATE meetings SET source_type = 'audio_import' WHERE id = ?1",
                    params![one.id]
                )
                .is_err());
        }
        let relative = PathBuf::from(format!("meetings/{}/audio/chunk.flac", one.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"audio").unwrap();
        let asset = storage
            .register_ready_meeting_asset(&one.id, &relative, "audio/flac")
            .unwrap();
        let db = storage.connection.lock().unwrap();
        let chunk_id = Uuid::new_v4().to_string();
        let insert_chunk = "INSERT INTO audio_chunks (id, meeting_id, track_id, file_asset_id, sequence, start_us, end_us, frame_count, sample_rate, channels, state, created_at) VALUES (?1, ?2, ?3, ?4, 0, 0, 1000000, 48000, 48000, 1, 'ready', 0)";
        assert!(db
            .execute(insert_chunk, params![chunk_id, two.id, track_id, asset.id])
            .is_err());
        db.execute(insert_chunk, params![chunk_id, one.id, track_id, asset.id])
            .unwrap();
        assert!(db
            .execute(
                insert_chunk,
                params![Uuid::new_v4().to_string(), one.id, track_id, asset.id]
            )
            .is_err());
        let problems: i64 = db
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(problems, 0);
    }

    #[test]
    fn transcript_and_evidence_references_are_version_scoped_and_frozen() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("content").unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute(
            "UPDATE meetings SET source_type = 'recording' WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
        db.execute("INSERT INTO recording_sessions (id, meeting_id, state, capture_mode, capture_config_json, start_operation_id, created_at, updated_at) VALUES ('session', ?1, 'saved', 'system', '{}', 'start', 0, 0)", params![meeting.id]).unwrap();
        db.execute("INSERT INTO audio_tracks (id, meeting_id, session_id, kind, format, sample_rate, channels, state, created_at, updated_at) VALUES ('track', ?1, 'session', 'mic', 'flac', 48000, 1, 'ready', 0, 0)", params![meeting.id]).unwrap();
        for (id, revision_no, hash) in [("analysis-1", 1, "b"), ("analysis-2", 2, "c")] {
            db.execute("INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, 'engine', 'candidate', ?4, '{}', '{}', ?5, 1, 0)", params![id, meeting.id, revision_no, "a".repeat(64), hash.repeat(64)]).unwrap();
        }
        db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('speaker-1', 'analysis-1', 'lineage-1', '話者1', 'all', 'unknown', 0)", []).unwrap();
        db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('speaker-2', 'analysis-2', 'lineage-2', '話者2', 'all', 'unknown', 0)", []).unwrap();
        db.execute("INSERT INTO utterances (id, meeting_id, analysis_revision_id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state) VALUES ('utterance-1', ?1, 'analysis-1', 'track', 'utterance-lineage', 0, 100, 1000, '発言', '発言', 'assigned')", params![meeting.id]).unwrap();
        assert!(db.execute("INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES ('utterance-1', 'speaker-2', 'analysis-1', 'primary', 'inferred')", []).is_err());
        db.execute("INSERT INTO utterance_speakers (utterance_id, speaker_id, analysis_revision_id, role, assignment_source) VALUES ('utterance-1', 'speaker-1', 'analysis-1', 'primary', 'inferred')", []).unwrap();
        assert!(db.execute("INSERT INTO speaker_segments (id, meeting_id, analysis_revision_id, track_id, speaker_id, start_us, end_us) VALUES ('segment-bad', ?1, 'analysis-1', 'track', 'speaker-2', 100, 1000)", params![meeting.id]).is_err());
        db.execute(
            "UPDATE analysis_revisions SET publication_state = 'committed' WHERE id = 'analysis-1'",
            [],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE utterances SET effective_text = '改変' WHERE id = 'utterance-1'",
                []
            )
            .is_err());
        assert!(db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('late', 'analysis-1', 'late', '後付け', 'all', 'unknown', 0)", []).is_err());
        db.execute("INSERT INTO minutes_revisions (id, meeting_id, input_analysis_id, revision_no, publication_state, generated_document_json, effective_document_json, document_schema_version, input_content_hash, content_hash, model_snapshot_json, template_version, created_at) VALUES ('minutes-1', ?1, 'analysis-1', 1, 'candidate', '{}', '{}', 1, ?2, ?3, '{}', 'v1', 0)", params![meeting.id, "b".repeat(64), "d".repeat(64)]).unwrap();
        let insert_item = "INSERT INTO minutes_items (id, minutes_revision_id, analysis_revision_id, kind, order_no, body, source_type) VALUES ('item-1', 'minutes-1', ?1, 'decision', 0, '決定', 'generated')";
        assert!(db.execute(insert_item, params!["analysis-2"]).is_err());
        db.execute(insert_item, params!["analysis-1"]).unwrap();
        let insert_evidence = "INSERT INTO evidence_links (id, minutes_revision_id, minutes_item_id, analysis_revision_id, utterance_id, start_us, end_us, source_type, verification_state) VALUES ('evidence-1', 'minutes-1', 'item-1', 'analysis-1', 'utterance-1', ?1, ?2, 'generated', 'unverified')";
        assert!(db.execute(insert_evidence, params![0, 1000]).is_err());
        db.execute(insert_evidence, params![100, 1000]).unwrap();
        db.execute(
            "UPDATE minutes_revisions SET publication_state = 'committed' WHERE id = 'minutes-1'",
            [],
        )
        .unwrap();
        assert!(db
            .execute(
                "UPDATE minutes_items SET body = '改変' WHERE id = 'item-1'",
                []
            )
            .is_err());
        assert!(db
            .execute("DELETE FROM evidence_links WHERE id = 'evidence-1'", [])
            .is_err());
        let problems: i64 = db
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(problems, 0);
    }
}
