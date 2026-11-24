//! The sole supported meeting-DB upgrade. The two schema39 origins are the
//! repository LF definition and the exact CRLF definition embedded in the
//! official Windows 1.0.7 artifact. Each retains its own SQL/provenance layout.
use super::*;

pub(super) const PREVIOUS_VERSION: usize = 39;
const PREVIOUS_CHECKSUM: &str = "0a59b9ce9524e3ed3e09d3a7e668d624e343f83b70cf0267cc33051eae42f86c";
const PREVIOUS_CRLF_CHECKSUM: &str =
    "d1d4559e7bf2aea303f783aaec6c5c5bd566b92f3b2f24e28ae373e28eb526c9";
const CURRENT_LF_CHECKSUM: &str =
    "c70486f1c7ac4522ed2d8abb23fbba0ba42b35485ac2a4d3c141b207502d954a";
const CURRENT_CRLF_CHECKSUM: &str =
    "4fca26c935d6c4edf430f6162ecf522f5110cc4d98279bf87d69e18f211ba185";
const OLD_PREDICATE: &str = "a.media_type='audio/x-caf'";
const NEW_PREDICATE: &str = "a.media_type IN ('audio/x-caf','audio/wav')";
const TRIGGER: &str = "audio_import_ready_assets";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Layout {
    Lf,
    Crlf,
}
impl Layout {
    fn previous_checksum(self) -> &'static str {
        match self {
            Self::Lf => PREVIOUS_CHECKSUM,
            Self::Crlf => PREVIOUS_CRLF_CHECKSUM,
        }
    }
    fn current_checksum(self) -> &'static str {
        match self {
            Self::Lf => CURRENT_LF_CHECKSUM,
            Self::Crlf => CURRENT_CRLF_CHECKSUM,
        }
    }
    fn newline(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

pub(super) fn current_sql(layout: Layout) -> Result<String, Box<dyn Error>> {
    let canonical = canonical_schema_sql();
    if canonical.matches(NEW_PREDICATE).count() != 1 || schema_checksum() != CURRENT_LF_CHECKSUM {
        return Err("schema40専用の定義チェックサムが一致しません".into());
    }
    let sql = match layout {
        Layout::Lf => canonical.to_owned(),
        Layout::Crlf => canonical.replace('\n', "\r\n"),
    };
    if mon_core::hash::sha256_hex(sql.as_bytes()) != layout.current_checksum() {
        return Err("schema40専用の改行形式が一致しません".into());
    }
    Ok(sql)
}
pub(super) fn previous_sql_for(layout: Layout) -> Result<String, Box<dyn Error>> {
    let previous = current_sql(layout)?.replacen(NEW_PREDICATE, OLD_PREDICATE, 1);
    if mon_core::hash::sha256_hex(previous.as_bytes()) != layout.previous_checksum() {
        return Err("schema39専用の定義チェックサムが一致しません".into());
    }
    Ok(previous)
}
#[derive(Debug)]
pub(super) struct UnsupportedSchema(String);

impl std::fmt::Display for UnsupportedSchema {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.0)
    }
}

impl Error for UnsupportedSchema {}

fn record(db: &Connection) -> Result<(i64, String), Box<dyn Error>> {
    let unsupported = "非対応の会議DB形式です。元のデータを保持しています";
    let mut query = db
        .prepare("SELECT version,checksum FROM schema_migrations ORDER BY version DESC LIMIT 2")
        .map_err(|error| -> Box<dyn Error> {
            // The statement is fixed. SQL errors such as a missing table or
            // column show an unsupported layout. Busy/permission/I/O errors
            // must retain their original SQLite code.
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::Unknown) {
                UnsupportedSchema(format!("{unsupported}: {error}")).into()
            } else {
                error.into()
            }
        })?;
    let mut rows = query.query([])?;
    let row = rows
        .next()?
        .ok_or_else(|| UnsupportedSchema(unsupported.into()))?;
    let record = match (row.get(0), row.get(1)) {
        (Ok(version), Ok(checksum)) => (version, checksum),
        (Err(error), _) | (_, Err(error)) => {
            if matches!(error, rusqlite::Error::InvalidColumnType(..)) {
                return Err(UnsupportedSchema(format!("{unsupported}: {error}")).into());
            }
            return Err(error.into());
        }
    };
    if rows.next()?.is_some() {
        return Err(UnsupportedSchema(unsupported.into()).into());
    }
    Ok(record)
}
pub(super) fn current_layout(db: &Connection) -> Result<Layout, Box<dyn Error>> {
    let (version, checksum) = record(db)?;
    if version == SCHEMA_VERSION as i64 && schema_checksum() == CURRENT_LF_CHECKSUM {
        if checksum == CURRENT_LF_CHECKSUM {
            return Ok(Layout::Lf);
        }
        if checksum == CURRENT_CRLF_CHECKSUM {
            return Ok(Layout::Crlf);
        }
    }
    Err(UnsupportedSchema(format!("非対応の会議DB形式です。元のデータを保持しています（記録された形式版: {version}、対応形式版: {SCHEMA_VERSION}）")).into())
}

type Object = (String, String, String, Option<String>);
fn objects(db: &Connection) -> Result<Vec<Object>, Box<dyn Error>> {
    let mut query = db.prepare(
        "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name",
    )?;
    let result = query
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(result)
}
fn verify_previous(db: &Connection) -> Result<Layout, Box<dyn Error>> {
    let (version, checksum) = record(db)?;
    let layout = match checksum.as_str() {
        PREVIOUS_CHECKSUM if version == PREVIOUS_VERSION as i64 => Layout::Lf,
        PREVIOUS_CRLF_CHECKSUM if version == PREVIOUS_VERSION as i64 => Layout::Crlf,
        _ => {
            return Err(UnsupportedSchema(
                "対応するschema39の記録・チェックサムではありません".into(),
            )
            .into())
        }
    };
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(&previous_sql_for(layout)?)?;
    if objects(db)? != objects(&expected)? {
        return Err(
            UnsupportedSchema("schema39の実際のSQL定義が正規の定義と一致しません".into()).into(),
        );
    }
    Ok(layout)
}

/// History/rebinding admission runs before writable startup migration. Its
/// exception requires query_only and an exact checksum/object-layout pair.
pub(super) fn verify_readonly_previous(db: &Connection) -> Result<Layout, Box<dyn Error>> {
    if !db.query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))? {
        return Err("schema39の履歴確認には読み取り専用接続が必要です".into());
    }
    verify_previous(db)
}

/// Only the expected deletion inventory uses this fingerprint. Every actual
/// SQL object is still compared to the exact checksum-selected40 layout. Keep
/// all original line endings; normalize only the known ready-import predicate.
pub(super) fn deletion_v1_schema_hash(
    objects: &[(String, String, String, String)],
) -> Result<String, String> {
    let mut legacy = objects.to_vec();
    let trigger = legacy
        .iter_mut()
        .find(|object| object.0 == "trigger" && object.1 == TRIGGER)
        .ok_or("削除v1の取り込みガードがありません")?;
    if trigger.3.matches(NEW_PREDICATE).count() != 1 {
        return Err("削除v1の既知の取り込みガードではありません".into());
    }
    trigger.3 = trigger.3.replacen(NEW_PREDICATE, OLD_PREDICATE, 1);
    Ok(mon_core::hash::sha256_hex(
        serde_json::to_vec(&legacy).map_err(|e| e.to_string())?,
    ))
}

pub(super) fn verify_startup(db: &Connection) -> Result<(), Box<dyn Error>> {
    match current_layout(db) {
        Ok(_) => Ok(()),
        Err(original) if original.is::<UnsupportedSchema>() => match verify_previous(db) {
            Ok(_) => Ok(()),
            Err(error) if error.is::<UnsupportedSchema>() => Err(original),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}
pub(super) fn migrate(db: &mut Connection) -> Result<(), Box<dyn Error>> {
    if current_layout(db).is_ok() {
        return Ok(());
    }
    let layout = verify_previous(db)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if verify_previous(&tx)? != layout {
        return Err("schema39の由来が移行中に変更されました".into());
    }
    let sql = current_sql(layout)?;
    let start = sql
        .find(&format!("CREATE TRIGGER {TRIGGER} "))
        .ok_or("schema40の取り込みガードがありません")?;
    let separator = format!("{}{}", layout.newline(), layout.newline());
    let end = start
        + sql[start..]
            .find(&separator)
            .ok_or("schema40の取り込みガードの区切りがありません")?;
    tx.execute_batch(&format!("DROP TRIGGER {TRIGGER};\n{}", &sql[start..end]))?;
    tx.execute(
        "DELETE FROM schema_migrations WHERE version=?1",
        [PREVIOUS_VERSION as i64],
    )?;
    tx.execute(
        "INSERT INTO schema_migrations(version,checksum,applied_at) VALUES(?1,?2,?3)",
        params![SCHEMA_VERSION as i64, layout.current_checksum(), now_ms()?],
    )?;
    verify_database_schema(&tx)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    #[test]
    fn fresh40_is_canonical_lf_and_layout_claims_do_not_admit_other_sql() {
        let mut fresh = Connection::open_in_memory().unwrap();
        initialize_database(&mut fresh).unwrap();
        assert_eq!(current_layout(&fresh).unwrap(), Layout::Lf);
        assert_eq!(record(&fresh).unwrap().1, CURRENT_LF_CHECKSUM);
        assert!(!canonical_schema_sql().contains('\r'));
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            migrate(&mut db).unwrap();
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            assert_eq!(current_layout(&db).unwrap(), layout);
            let expected = Connection::open_in_memory().unwrap();
            expected
                .execute_batch(&current_sql(layout).unwrap())
                .unwrap();
            assert_eq!(objects(&db).unwrap(), objects(&expected).unwrap());
            deletion::database::trusted_schema(&db).unwrap();
            let other = if layout == Layout::Lf {
                Layout::Crlf
            } else {
                Layout::Lf
            };
            db.execute(
                "UPDATE schema_migrations SET checksum=?1",
                [other.current_checksum()],
            )
            .unwrap();
            assert!(
                deletion::database::trusted_schema(&db).is_err(),
                "wrong-layout provenance"
            );
            db.execute(
                "UPDATE schema_migrations SET checksum=?1",
                [layout.current_checksum()],
            )
            .unwrap();
            db.execute_batch("CREATE TABLE unknown_source(id INTEGER)")
                .unwrap();
            assert!(
                deletion::database::trusted_schema(&db).is_err(),
                "unknown SQL"
            );
        }
    }

    fn previous_for(layout: Layout) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&previous_sql_for(layout).unwrap())
            .unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(39,?1,0)",
            [layout.previous_checksum()],
        )
        .unwrap();
        db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES('retained','保持する旧会議','draft',0,0)",[]).unwrap();
        db
    }

    #[test]
    fn known39_checksum_cannot_relabel_the_other_layout() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            let other = if layout == Layout::Lf {
                Layout::Crlf
            } else {
                Layout::Lf
            };
            db.execute(
                "UPDATE schema_migrations SET checksum=?1",
                [other.previous_checksum()],
            )
            .unwrap();
            let before = objects(&db).unwrap();
            assert!(verify_startup(&db).is_err());
            assert!(migrate(&mut db).is_err());
            assert_eq!(objects(&db).unwrap(), before);
            assert_eq!(record(&db).unwrap().0, 39);
        }
    }

    #[test]
    fn exact_schema39_upgrades_only_its_guard_and_preserves_rows() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            verify_startup(&db).unwrap();
            let before = objects(&db).unwrap();
            migrate(&mut db).unwrap();
            verify_database_schema(&db).unwrap();
            let after = objects(&db).unwrap();
            let changed: Vec<_> = before
                .iter()
                .zip(&after)
                .filter(|(old, new)| old != new)
                .collect();
            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].0 .1, TRIGGER);
            assert_eq!(
                db.query_row(
                    "SELECT title FROM meetings WHERE id='retained'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                "保持する旧会議"
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM schema_migrations", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            migrate(&mut db).unwrap();
            assert_eq!(objects(&db).unwrap(), after);
        }
    }

    #[test]
    fn storage_startup_upgrades_real39_file_and_keeps_existing_audio() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let root = tempfile::tempdir().unwrap();
            let db = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
            db.execute_batch(&previous_sql_for(layout).unwrap())
                .unwrap();
            db.execute(
                "INSERT INTO schema_migrations VALUES(39,?1,0)",
                [layout.previous_checksum()],
            )
            .unwrap();
            let meeting = Uuid::new_v4().to_string();
            db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,'旧データ','draft',0,0)", [&meeting]).unwrap();
            drop(db);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    root.path().join("meetings.sqlite3"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            #[cfg(windows)]
            crate::windows_files::set_private(&root.path().join("meetings.sqlite3")).unwrap();
            let audio = root.path().join("retained.caf");
            std::fs::write(&audio, b"owned fixture audio unchanged").unwrap();
            let storage = Storage::open(root.path().to_owned()).unwrap();
            verify_database_schema(&storage.connection.lock().unwrap()).unwrap();
            assert_eq!(
                storage.get_meeting(&meeting).unwrap().unwrap().title,
                "旧データ"
            );
            assert_eq!(
                std::fs::read(audio).unwrap(),
                b"owned fixture audio unchanged"
            );
        }
    }

    #[test]
    fn forged_old_checksum_or_changed_sql_is_rejected_without_writes() {
        for layout in [Layout::Lf, Layout::Crlf] {
            for change in [
                "UPDATE schema_migrations SET checksum='changed'",
                "DROP TRIGGER audio_import_ready_assets",
                "CREATE TABLE unknown_source(id INTEGER)",
                "INSERT INTO schema_migrations VALUES(38,'other',0)",
            ] {
                let mut db = previous_for(layout);
                db.execute_batch(change).unwrap();
                let before = objects(&db).unwrap();
                assert!(verify_startup(&db).is_err(), "{change}");
                assert!(migrate(&mut db).is_err(), "{change}");
                assert_eq!(objects(&db).unwrap(), before);
                assert_eq!(
                    db.query_row("SELECT max(version) FROM schema_migrations", [], |row| row
                        .get::<_, i64>(
                        0
                    ))
                    .unwrap(),
                    39
                );
            }
        }
    }

    #[test]
    fn migration_failure_rolls_back_guard_and_provenance_together() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            let before = objects(&db).unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Insert {
                    table_name: "schema_migrations",
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
            assert!(migrate(&mut db).is_err());
            db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert_eq!(objects(&db).unwrap(), before);
            verify_previous(&db).unwrap();
        }
    }

    #[test]
    fn deletion_v1_fingerprint_matches_real39_but_sql40_remains_exact() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            let old: Vec<(String, String, String, String)> = objects(&db)
                .unwrap()
                .into_iter()
                .map(|(kind, name, table, sql)| (kind, name, table, sql.unwrap()))
                .collect();
            let fingerprint = mon_core::hash::sha256_hex(serde_json::to_vec(&old).unwrap());
            migrate(&mut db).unwrap();
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            assert_eq!(
                deletion::database::trusted_schema(&db).unwrap(),
                fingerprint
            );
            verify_database_schema(&db).unwrap();
            db.execute_batch("CREATE TABLE unknown_source(id INTEGER)")
                .unwrap();
            assert!(deletion::database::trusted_schema(&db).is_err());
        }
    }

    #[test]
    fn old_history_admission_requires_query_only_and_exact39_sql() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            assert!(deletion::database::trusted_schema(&db).is_err());
            db.execute_batch("PRAGMA query_only=ON").unwrap();
            let historical = deletion::database::trusted_schema(&db).unwrap();
            db.execute_batch("PRAGMA query_only=OFF").unwrap();
            assert!(deletion::database::trusted_schema(&db).is_err());
            migrate(&mut db).unwrap();
            assert_eq!(deletion::database::trusted_schema(&db).unwrap(), historical);
            let old = previous_for(layout);
            old.execute_batch("DROP TRIGGER audio_import_ready_assets; PRAGMA query_only=ON;")
                .unwrap();
            assert!(deletion::database::trusted_schema(&old).is_err());
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn recovery_pre_migration_validation_restores_connection_on_success_and_error() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let mut db = previous_for(layout);
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            assert!(Storage::validate_rebinding(&db).is_err());
            assert!(Storage::validate_rebinding_before_migration(&db)
                .unwrap()
                .is_empty());
            assert!(!db
                .query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))
                .unwrap());
            db.execute("UPDATE meetings SET title='復旧後も書込み可能'", [])
                .unwrap();
            migrate(&mut db).unwrap();
            verify_database_schema(&db).unwrap();

            let old = previous_for(layout);
            old.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON")
                .unwrap();
            Storage::validate_rebinding_before_migration(&old).unwrap();
            assert!(old
                .query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))
                .unwrap());
            old.execute_batch("PRAGMA query_only=OFF; DROP TRIGGER audio_import_ready_assets")
                .unwrap();
            assert!(Storage::validate_rebinding_before_migration(&old).is_err());
            assert!(!old
                .query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))
                .unwrap());
            old.execute("UPDATE meetings SET title='失敗後も元の接続状態'", [])
                .unwrap();
        }
    }
}
