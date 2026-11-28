//! 解析診断を追加する会議DBの移行。
use super::{schema_40::Layout, *};

pub(super) const PREVIOUS_VERSION: usize = 40;
pub(super) const TABLE: &str = "analysis_vocabulary_diagnostics";
const START: &str = "CREATE TABLE analysis_vocabulary_diagnostics (";
const END: &str = "CREATE TABLE analysis_flows (";

fn addition() -> &'static str {
    let sql = canonical_schema_sql();
    let start = sql.find(START).expect("解析診断のSQL定義");
    let end = sql.find(END).expect("解析フローのSQL定義");
    &sql[start..end]
}

pub(super) fn schema_40_sql() -> String {
    canonical_schema_sql().replacen(addition(), "", 1)
}

pub(super) fn current_sql(layout: Layout) -> String {
    layout.with_line_endings(canonical_schema_sql())
}

pub(super) fn current_layout(db: &Connection) -> Result<Layout, Box<dyn Error>> {
    let (version, checksum) = schema_40::record(db)?;
    if version == SCHEMA_VERSION as i64 {
        for layout in [Layout::Lf, Layout::Crlf] {
            if checksum == checksum_for(layout) {
                return Ok(layout);
            }
        }
    }
    Err(schema_40::UnsupportedSchema(format!("非対応の会議DB形式です。元のデータを保持しています（記録された形式版: {version}、対応形式版: {SCHEMA_VERSION}）")).into())
}

pub(super) fn checksum_for(layout: Layout) -> String {
    match layout {
        Layout::Lf => schema_checksum().to_owned(),
        Layout::Crlf => mon_core::hash::sha256_hex(current_sql(layout).as_bytes()),
    }
}

fn verify_40(db: &Connection) -> Result<Layout, Box<dyn Error>> {
    let layout = schema_40::current_layout(db)?;
    verify_objects(db, &schema_40::current_sql(layout)?)?;
    Ok(layout)
}

fn verify_objects(db: &Connection, sql: &str) -> Result<(), Box<dyn Error>> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(sql)?;
    salvage_schema::verify(db)?;
    let mut actual = schema_40::objects(db)?;
    actual.retain(|(_, name, _, _)| !salvage_schema::owns_object(name));
    if actual != schema_40::objects(&expected)? {
        return Err(schema_40::UnsupportedSchema(
            "会議DBのSQL定義が正規の定義と一致しません".into(),
        )
        .into());
    }
    Ok(())
}

pub(super) fn verify_startup(db: &Connection) -> Result<(), Box<dyn Error>> {
    match current_layout(db) {
        Ok(_) => Ok(()),
        Err(error) if error.is::<schema_40::UnsupportedSchema>() => {
            match schema_40::record(db)?.0 {
                40 => verify_40(db).map(|_| ()),
                39 => schema_40::verify_startup(db),
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

pub(super) fn verify_readonly_previous(db: &Connection) -> Result<(usize, Layout), Box<dyn Error>> {
    if !db.query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))? {
        return Err("旧会議DBの履歴確認には読み取り専用接続が必要です".into());
    }
    if schema_40::record(db)?.0 == PREVIOUS_VERSION as i64 {
        Ok((PREVIOUS_VERSION, verify_40(db)?))
    } else {
        Ok((
            schema_40::PREVIOUS_VERSION,
            schema_40::verify_readonly_previous(db)?,
        ))
    }
}

pub(super) fn migrate(db: &mut Connection) -> Result<(), Box<dyn Error>> {
    if current_layout(db).is_ok() {
        return Ok(());
    }
    if schema_40::record(db)?.0 == schema_40::PREVIOUS_VERSION as i64 {
        schema_40::migrate(db)?;
    }
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if current_layout(&tx).is_ok() {
        return Ok(());
    }
    let layout = verify_40(&tx)?;
    tx.execute_batch(&layout.with_line_endings(addition()))?;
    tx.execute(
        "UPDATE schema_migrations SET version=?1,checksum=?2,applied_at=?3 WHERE version=?4",
        params![
            SCHEMA_VERSION as i64,
            checksum_for(layout),
            now_ms()?,
            PREVIOUS_VERSION as i64
        ],
    )?;
    verify_objects(&tx, &current_sql(layout))?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    #[test]
    fn schema_41_startup_preserves_schema_40_analysis_and_retries_rollback() {
        for layout in [Layout::Lf, Layout::Crlf] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("meetings.sqlite3");
            let mut db = Connection::open(&path).unwrap();
            let sql = schema_40::current_sql(layout).unwrap();
            db.execute_batch(&sql).unwrap();
            db.execute(
                "INSERT INTO schema_migrations VALUES(40,?1,0)",
                [mon_core::hash::sha256_hex(sql.as_bytes())],
            )
            .unwrap();
            let meeting = Uuid::new_v4().to_string();
            db.execute("INSERT INTO meetings(id,title,lifecycle,created_at,updated_at) VALUES(?1,'旧会議','draft',0,0)", [&meeting]).unwrap();
            let tx = db.transaction().unwrap();
            tx.execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES('analysis',?1,1,'engine','building',?2,'{}','{}',?2,1,0)", params![meeting,"a".repeat(64)]).unwrap();
            let hash = revisions::analysis_content_hash(&tx, "analysis").unwrap();
            tx.execute("UPDATE analysis_revisions SET content_hash=?1,publication_state='committed' WHERE id='analysis'", [&hash]).unwrap();
            tx.execute(
                "UPDATE meetings SET current_analysis_id='analysis' WHERE id=?1",
                [&meeting],
            )
            .unwrap();
            tx.commit().unwrap();
            db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Update {
                    table_name: "schema_migrations",
                    ..
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
            assert!(migrate(&mut db).is_err());
            db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert_eq!(verify_40(&db).unwrap(), layout);
            assert!(schema_40::verify_startup(&db).is_ok());
            drop(db);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            #[cfg(windows)]
            crate::windows_files::set_private(&path).unwrap();
            let storage = Storage::open(root.path().to_owned()).unwrap();
            let analysis = storage
                .load_current_analysis_presentation(&meeting)
                .unwrap()
                .unwrap();
            assert_eq!(analysis.analysis.id, "analysis");
            assert_eq!(analysis.analysis.content_hash, hash);
            assert!(analysis.ignored_hotwords.is_empty());
            let mut db = storage.connection.lock().unwrap();
            assert_eq!(current_layout(&db).unwrap(), layout);
            assert!(schema_40::verify_startup(&db).is_err());
            migrate(&mut db).unwrap();
            verify_objects(&db, &current_sql(layout)).unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
                0
            );
        }
    }
}
