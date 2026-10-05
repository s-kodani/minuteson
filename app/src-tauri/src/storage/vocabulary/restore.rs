//! Additive restore of the current global book, never its frozen snapshots.
use super::*;
use rusqlite::Transaction;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::storage) struct VocabularyConflict {
    pub term: String,
    pub existing_id: String,
    pub restored_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(in crate::storage) struct MergeReceipt {
    pub added: usize,
    pub deduplicated: usize,
    pub conflicts: Vec<VocabularyConflict>,
}

pub(in crate::storage) struct GlobalMerge {
    expected: String,
    additions: Vec<StoredEntry>,
    receipt: MergeReceipt,
}

fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("共通用語の追加復元を中断しました".into())
    } else {
        Ok(())
    }
}

fn book(db: &Connection) -> Result<(VocabularyBook, Vec<StoredEntry>), String> {
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM vocabulary_entries WHERE scope_id='global'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !(0..=500).contains(&count) {
        return Err("共通用語の件数が上限を超えています".into());
    }
    load(db, "global")
}

fn equal_content(a: &VocabularyEntry, b: &VocabularyEntry) -> bool {
    a.term == b.term && a.reading == b.reading && a.note == b.note && a.enabled == b.enabled
}

impl GlobalMerge {
    pub(in crate::storage) fn plan(
        source: &Connection,
        destination: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancelled(check)?;
        let (old, existing) = book(destination)?;
        let (_, incoming) = book(source)?;
        let mut receipt = MergeReceipt {
            added: 0,
            deduplicated: 0,
            conflicts: vec![],
        };
        let by_term: HashMap<_, _> = old
            .payload
            .iter()
            .map(|entry| (entry.term.as_str(), entry))
            .collect();
        let mut additions = vec![];
        for stored in incoming {
            cancelled(check)?;
            let entry = &stored.0;
            let collision: bool = destination
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM vocabulary_entries WHERE id=?1)",
                    params![entry.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if collision {
                return Err("復元する共通用語IDが既存データと競合しています".into());
            }
            if let Some(prior) = by_term.get(entry.term.as_str()) {
                if equal_content(prior, entry) {
                    receipt.deduplicated += 1;
                } else {
                    receipt.conflicts.push(VocabularyConflict {
                        term: entry.term.clone(),
                        existing_id: prior.id.clone(),
                        restored_id: entry.id.clone(),
                    });
                }
            } else {
                additions.push(stored);
            }
        }
        if existing.len() + additions.len() > 500 {
            return Err("共通用語の追加後に500件を超えます。既存を維持し、復元を停止します".into());
        }
        receipt.added = additions.len();
        cancelled(check)?;
        Ok(Self {
            expected: hash(&(&existing, old.lock_version))?,
            additions,
            receipt,
        })
    }

    pub(in crate::storage) fn receipt(&self) -> &MergeReceipt {
        &self.receipt
    }

    pub(in crate::storage) fn confirmation_fingerprint(&self) -> &str {
        &self.expected
    }

    // Publication owns the outer transaction. Even if it commits after an
    // error, this merge cannot leave a scope or a subset of additions behind.
    pub(in crate::storage) fn apply(
        &self,
        tx: &mut Transaction<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<MergeReceipt, String> {
        let savepoint = tx.savepoint().map_err(|e| e.to_string())?;
        let receipt = self.apply_inner(&savepoint, check)?;
        savepoint.commit().map_err(|e| e.to_string())?;
        Ok(receipt)
    }

    fn apply_inner(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<MergeReceipt, String> {
        cancelled(check)?;
        let (old, mut current) = book(db)?;
        if hash(&(&current, old.lock_version))? != self.expected {
            return Err("共通用語が復元確認後に変更されています。再確認してください".into());
        }
        if self.additions.is_empty() {
            return Ok(self.receipt.clone());
        }
        let version = old
            .lock_version
            .checked_add(1)
            .ok_or("共通用語の版番号が範囲外です")?;
        let now = now_ms()?;
        let mut position: i64 = db
            .query_row(
                "SELECT coalesce(max(position),-1) FROM vocabulary_entries WHERE scope_id='global'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        db.execute("INSERT INTO vocabulary_scopes(id,meeting_id,content_hash,lock_version,updated_at) VALUES('global',NULL,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params!["0".repeat(64),version,now]).map_err(|e|e.to_string())?;
        for row in &self.additions {
            cancelled(check)?;
            position = position
                .checked_add(1)
                .ok_or("共通用語の位置が範囲外です")?;
            let e = &row.0;
            db.execute("INSERT INTO vocabulary_entries(id,scope_id,position,term,reading,note,enabled,lock_version,created_at,updated_at) VALUES(?1,'global',?2,?3,?4,?5,?6,?7,?8,?9)",params![e.id,position,e.term,e.reading,e.note,e.enabled,row.1,row.2,row.3]).map_err(|e|e.to_string())?;
            current.push(row.clone());
        }
        db.execute(
            "UPDATE vocabulary_scopes SET content_hash=?1 WHERE id='global'",
            params![hash(&(&current, version))?],
        )
        .map_err(|e| e.to_string())?;
        verify_saved_books(db)?;
        cancelled(check)?;
        Ok(self.receipt.clone())
    }
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
