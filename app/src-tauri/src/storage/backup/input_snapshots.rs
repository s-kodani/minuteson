//! A recording's immutable logical evidence is owned by its DB row, not by an
//! invented capture directory or a mutable fixed-hash file asset. Producer IDs
//! stay intact; additive restoration rebinds only the containing logical owner.
use super::*;
use crate::input_status::queue::PortableInputSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(in crate::storage) const MAX_BYTES: usize = 513 * 1024;
const MAX_RECORDINGS: usize = 10_000;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct Bound {
    schema_version: u8,
    meeting_id: String,
    session_id: String,
    snapshot: PortableInputSnapshot,
}
impl Bound {
    pub(in crate::storage) fn into_snapshot(self) -> PortableInputSnapshot {
        self.snapshot
    }

    fn validate(&self, meeting: &str, session: &str) -> Result<(), String> {
        if self.schema_version != 1
            || self.meeting_id != meeting
            || self.session_id != session
            || !crate::input_levels::valid_owner(meeting)
            || !crate::input_levels::valid_owner(session)
        {
            return Err("入力履歴スナップショットの会議・録音所属が不正です".into());
        }
        let data = self.snapshot.encode()?;
        PortableInputSnapshot::decode(&data, self.snapshot.source_owner_id())?;
        Ok(())
    }
    pub(in crate::storage) fn decode(
        text: &str,
        meeting: &str,
        session: &str,
    ) -> Result<Self, String> {
        if text.is_empty() || text.len() > MAX_BYTES {
            return Err("入力履歴スナップショットの容量が不正です".into());
        }
        let bound: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        // Original typed bytes reject duplicate known keys at every level;
        // equality also requires explicit optional fields, including nulls.
        if serde_json::to_value(&bound).map_err(|e| e.to_string())?
            != serde_json::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())?
        {
            return Err("入力履歴スナップショットの必須項目が不正です".into());
        }
        bound.validate(meeting, session)?;
        Ok(bound)
    }
    fn encode(&self) -> Result<String, String> {
        self.validate(&self.meeting_id, &self.session_id)?;
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if text.len() > MAX_BYTES {
            return Err("入力履歴スナップショットが容量上限を超えました".into());
        }
        Ok(text)
    }
    pub(super) fn rebind(self, meeting: &str, session: &str) -> Result<String, String> {
        Self {
            meeting_id: meeting.into(),
            session_id: session.into(),
            ..self
        }
        .encode()
    }
}

fn has_column(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('recording_sessions') WHERE name='input_history_snapshot_json')", [], |r| r.get(0)).map_err(|e|e.to_string())
}

pub(super) fn validate_database(
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    super::super::restored_input::validate_database(db, cancelled)?;
    if !has_column(db)? {
        return Ok(());
    } // Trusted pre-29 schema has no evidence.
    let mut query = db.prepare("SELECT meeting_id,id,state,input_history_snapshot_json FROM recording_sessions WHERE input_history_snapshot_json IS NOT NULL ORDER BY id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut count = 0_usize;
    let mut total = 0_usize;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        let (meeting, session, state, text): (String, String, String, String) = (
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            row.get(3).map_err(|e| e.to_string())?,
        );
        count += 1;
        total = total
            .checked_add(text.len())
            .filter(|n| *n <= MAX_TOTAL_BYTES)
            .ok_or("入力履歴スナップショットの総容量が安全上限を超えました")?;
        if count > MAX_RECORDINGS {
            return Err("入力履歴スナップショットの件数が安全上限を超えました".into());
        }
        if !matches!(state.as_str(), "saved" | "interrupted" | "failed") {
            return Err("録音終了前の入力履歴スナップショットは採用しません".into());
        }
        Bound::decode(&text, &meeting, &session)?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    super::super::restored_input::remove_schema_for_test(db);
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=29)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(db.query_row("SELECT count(*) FROM recording_sessions WHERE input_history_snapshot_json IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        db.execute_batch("DROP TRIGGER recording_input_snapshot_owner; DROP TRIGGER recording_input_snapshot_binding; DROP TRIGGER recording_input_snapshot_fixed; ALTER TABLE recording_sessions DROP COLUMN input_history_snapshot_json; DELETE FROM schema_migrations WHERE version=29;").unwrap();
    }
}

pub(super) struct Native {
    payloads: BTreeMap<(String, String), String>,
    transfers: BTreeMap<(String, String), String>,
    #[cfg(any(unix, windows))]
    root: File,
    #[cfg(any(unix, windows))]
    witnesses: Vec<source::Witness>,
}
impl Native {
    pub(super) fn capture(
        storage: &Storage,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        validate_database(db, cancelled)?;
        let mut result = Self {
            payloads: BTreeMap::new(),
            transfers: BTreeMap::new(),
            #[cfg(any(unix, windows))]
            root: storage.managed_anchor.backup_input_root()?,
            #[cfg(any(unix, windows))]
            witnesses: Vec::new(),
        };
        let mut query = db.prepare("SELECT meeting_id,id,input_history_snapshot_json FROM recording_sessions WHERE state IN ('saved','interrupted','failed') ORDER BY id LIMIT 10001").map_err(|e|e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        let mut count = 0;
        let mut total = 0_usize;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            count += 1;
            if count > MAX_RECORDINGS {
                return Err("バックアップする録音履歴の件数が安全上限を超えました".into());
            }
            let (meeting, session, old): (String, String, Option<String>) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
            );
            let payload = if let Some(text) = old {
                // A restored immutable snapshot is not permission to inspect
                // guessed native audio paths or to create a capture owner.
                let bound = Bound::decode(&text, &meeting, &session)?;
                if let Some(transfer) = super::super::restored_input::capture_transfer(
                    db,
                    &meeting,
                    &session,
                    &bound.snapshot,
                )? {
                    total = total
                        .checked_add(transfer.len())
                        .filter(|n| *n <= MAX_TOTAL_BYTES)
                        .ok_or("録音入力の移送記録の総容量が安全上限を超えました")?;
                    result
                        .transfers
                        .insert((meeting.clone(), session.clone()), transfer);
                }
                Some(bound.encode()?)
            } else {
                #[cfg(any(unix, windows))]
                {
                    // Old logical fixtures/legacy IDs have no native owner
                    // capability. Never turn them into path components.
                    if crate::input_levels::valid_owner(&meeting)
                        && crate::input_levels::valid_owner(&session)
                    {
                        let (witness, snapshot) =
                            source::capture(&result.root, &meeting, &session, cancelled)?;
                        result.witnesses.push(witness);
                        snapshot
                            .map(|snapshot| {
                                Bound {
                                    schema_version: 1,
                                    meeting_id: meeting.clone(),
                                    session_id: session.clone(),
                                    snapshot,
                                }
                                .encode()
                            })
                            .transpose()?
                    } else {
                        None
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = storage;
                    None
                }
            };
            if let Some(text) = payload {
                total = total
                    .checked_add(text.len())
                    .filter(|n| *n <= MAX_TOTAL_BYTES)
                    .ok_or("入力履歴スナップショットの総容量が安全上限を超えました")?;
                result.payloads.insert((meeting, session), text);
            }
        }
        result.verify(storage, cancelled)?;
        Ok(result)
    }
    pub(super) fn payload(&self, meeting: &str, session: &str) -> Option<&str> {
        self.payloads
            .get(&(meeting.into(), session.into()))
            .map(String::as_str)
    }
    pub(super) fn transfer(&self, meeting: &str, session: &str) -> Option<&str> {
        self.transfers
            .get(&(meeting.into(), session.into()))
            .map(String::as_str)
    }
    pub(super) fn verify(
        &self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        #[cfg(any(unix, windows))]
        {
            storage.managed_anchor.verify()?;
            for witness in &self.witnesses {
                witness.verify(&self.root, cancelled)?;
            }
            storage.managed_anchor.verify()?;
        }
        #[cfg(not(any(unix, windows)))]
        let _ = storage;
        Ok(())
    }
}

#[cfg(any(unix, windows))]
mod source;
#[cfg(all(test, unix))]
mod tests;
