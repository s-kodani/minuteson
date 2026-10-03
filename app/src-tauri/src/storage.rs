use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MIGRATION_1: &str = include_str!("migrations/0001_meetings.sql");

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

pub struct Storage {
    root: PathBuf,
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(root: PathBuf) -> Result<Self, Box<dyn Error>> {
        std::fs::create_dir_all(&root)?;
        let mut connection = Connection::open(root.join("meetings.sqlite3"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        apply_migrations(&mut connection)?;
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
    let digest = format!("{:x}", Sha256::digest(MIGRATION_1.as_bytes()));
    for (version, checksum) in &known {
        if *version != 1 || checksum != &digest {
            return Err(format!("unsupported or changed migration {version}").into());
        }
    }
    if known.is_empty() {
        let transaction = connection.transaction()?;
        transaction.execute_batch(MIGRATION_1)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, checksum, applied_at) VALUES (1, ?1, ?2)",
            params![digest, now_ms()?],
        )?;
        transaction.commit()?;
    }
    Ok(())
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
}
