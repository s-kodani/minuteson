//! Preserve previous origins and append this operation's old/new key history.
use super::*;

impl FixedWorkspace {
    pub(super) fn provenance(
        &mut self,
        fixed: &FixedData<'_, '_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let source = &fixed.ids.source().database;
        let tables = portable::tables(source)?;
        if let Some(table) = tables.iter().find(|t| t.name == "restore_provenance") {
            let mut query = source
                .prepare("SELECT rowid,id,meeting_id FROM restore_provenance ORDER BY rowid")
                .map_err(|e| e.to_string())?;
            let mut rows = query.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                let rowid: i64 = row.get(0).map_err(|e| e.to_string())?;
                let id: String = row.get(1).map_err(|e| e.to_string())?;
                let meeting: Option<String> = row.get(2).map_err(|e| e.to_string())?;
                let id =
                    fixed
                        .ids
                        .get("restore_provenance", meeting.as_deref().unwrap_or(""), &id)?;
                let actual: Vec<rusqlite::types::Value> = self
                    .database
                    .query_row(
                        "SELECT * FROM restore_provenance WHERE id=?1",
                        params![id],
                        |r| (0..table.columns.len()).map(|i| r.get(i)).collect(),
                    )
                    .map_err(|e| e.to_string())?;
                if actual != fixed.typed_row(table, rowid)? {
                    return Err("構築済みの復元由来が変更されています".into());
                }
            }
        }
        let now = now_ms()?;
        let tx = self.database.transaction().map_err(|e| e.to_string())?;
        fixed.ids.visit_mappings(cancelled, |map| {
            if map.domain == "restore_provenance" {
                return Ok(());
            }
            if !super::super::super::ledger::valid_domain(&map.domain) {
                return Err("復元由来に未知のID領域があります".into());
            }
            let meeting = if map.domain == "meetings" {
                Some(map.target.clone())
            } else if map.scope.is_empty() {
                None
            } else {
                Some(fixed.ids.get("meetings", "", &map.scope)?)
            };
            let id = loop {
                check_cancelled(cancelled)?;
                let id = Uuid::new_v4().to_string();
                let present: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM restore_provenance WHERE id=?1)",
                        params![id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !present && fixed.ids.unused_identity(&id)? {
                    break id;
                }
            };
            tx.execute(
                "INSERT INTO restore_provenance VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    id,
                    meeting,
                    map.domain,
                    map.target,
                    fixed.ids.archive_id,
                    map.scope,
                    map.source,
                    fixed.ids.batch_id,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })?;
        super::super::super::ledger::validate_provenance(&tx, cancelled)?;
        check_cancelled(cancelled)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
