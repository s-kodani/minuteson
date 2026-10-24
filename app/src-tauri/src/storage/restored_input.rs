//! One destination writer per Storage incarnation, shared by all archived-input
//! controllers. SQLite is the durable owner; no producer directory is invented.
use super::*;
use crate::{
    analysis_worker::data_maintenance::NoticeStorageLease,
    input_status::{
        queue::{Action, ArchivedBatch, PortableInputSnapshot, RestoredInputState},
        Receipt,
    },
};
use std::{collections::BTreeMap, time::Instant};

pub(in crate::storage) const TRANSFER_MAX_BYTES: usize = 257 * 1024;
const STATE_MAX_BYTES: usize = 256 * 1024;
const CACHE_LIMIT: usize = 128;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct Transfer {
    schema_version: u8,
    meeting_id: String,
    session_id: String,
    state: RestoredInputState,
}
impl Transfer {
    pub(in crate::storage) fn decode(
        text: &str,
        meeting: &str,
        owner: &str,
        source: &PortableInputSnapshot,
    ) -> Result<Self, String> {
        if text.is_empty() || text.len() > TRANSFER_MAX_BYTES {
            return Err("復元通知の移送記録の容量が不正です".into());
        }
        let value: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if value.schema_version != 1
            || value.meeting_id != meeting
            || value.session_id != owner
            || !crate::input_levels::valid_owner(meeting)
            || !crate::input_levels::valid_owner(owner)
            || serde_json::to_value(&value).map_err(|e| e.to_string())?
                != serde_json::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())?
        {
            return Err("復元通知の移送記録の所属または項目が不正です".into());
        }
        // The inner owner is historical delivery evidence, not the restored
        // logical owner. Its claims are fenced before destination initialization.
        RestoredInputState::decode(&value.state.encode(source)?, source, value.state.owner())?;
        Ok(value)
    }
    fn encode(&self, source: &PortableInputSnapshot) -> Result<String, String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        Self::decode(&text, &self.meeting_id, &self.session_id, source)?;
        Ok(text)
    }
    pub(in crate::storage) fn rebind(
        mut self,
        meeting: &str,
        owner: &str,
        source: &PortableInputSnapshot,
    ) -> Result<String, String> {
        self.meeting_id = meeting.into();
        self.session_id = owner.into();
        self.encode(source)
    }
}

struct Scope {
    source: PortableInputSnapshot,
    transfer: Option<Transfer>,
    identity: String,
}
impl Scope {
    fn read(db: &Connection, meeting: &str, owner: &str) -> Result<Self, String> {
        if !crate::input_levels::valid_owner(meeting) || !crate::input_levels::valid_owner(owner) {
            return Err("復元通知の会議または録音所属が不正です".into());
        }
        type Row = (
            String,
            String,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<String>,
        );
        let row: Option<Row> = db.query_row(
            "SELECT meeting_id,state,length(CAST(input_history_snapshot_json AS BLOB)),CASE WHEN length(CAST(input_history_snapshot_json AS BLOB)) <= ?2 THEN input_history_snapshot_json END,length(CAST(input_delivery_transfer_json AS BLOB)),CASE WHEN length(CAST(input_delivery_transfer_json AS BLOB)) <= ?3 THEN input_delivery_transfer_json END FROM recording_sessions WHERE id=?1",
            params![owner, backup::input_snapshots::MAX_BYTES as i64, TRANSFER_MAX_BYTES as i64],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|e.to_string())?;
        let (actual, state, size, source_text, transfer_size, transfer_text) =
            row.ok_or("復元通知の録音が見つかりません")?;
        if actual != meeting || !matches!(state.as_str(), "saved" | "interrupted" | "failed") {
            return Err("復元通知の会議または終了状態が変更されています".into());
        }
        let source_text = match (size, source_text) {
            (Some(n), Some(text)) if n > 0 => text,
            _ => return Err("復元通知の元記録がないか容量が不正です".into()),
        };
        let source =
            backup::input_snapshots::Bound::decode(&source_text, meeting, owner)?.into_snapshot();
        let transfer = match (transfer_size, &transfer_text) {
            (None, None) => None,
            (Some(n), Some(text)) if n > 0 => {
                Some(Transfer::decode(text, meeting, owner, &source)?)
            }
            _ => return Err("復元通知の移送記録の容量が不正です".into()),
        };
        let mut hash = Sha256::new();
        hash.update(source_text.as_bytes());
        hash.update([0]);
        if let Some(text) = transfer_text {
            hash.update(text.as_bytes());
        }
        Ok(Self {
            source,
            transfer,
            identity: format!("{:x}", hash.finalize()),
        })
    }
}

struct Durable {
    writer: String,
    state: RestoredInputState,
    text: String,
}
fn durable(
    db: &Connection,
    meeting: &str,
    owner: &str,
    source: &PortableInputSnapshot,
) -> Result<Option<Durable>, String> {
    type Row = (String, String, i64, i64, Option<String>);
    let row: Option<Row> = db.query_row(
        "SELECT meeting_id,writer_id,revision,length(CAST(state_json AS BLOB)),CASE WHEN length(CAST(state_json AS BLOB)) <= ?2 THEN state_json END FROM recording_input_delivery WHERE session_id=?1",
        params![owner, STATE_MAX_BYTES as i64], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
    row.map(|(actual, writer, revision, size, text)| {
        if actual != meeting || !crate::input_levels::valid_owner(&writer) || size <= 0 {
            return Err("復元通知台帳の所属または書込み所有が不正です".into());
        }
        let text = text.ok_or("復元通知台帳の容量が上限を超えました")?;
        let state = RestoredInputState::decode(text.as_bytes(), source, owner)?;
        if revision != state.revision() as i64 {
            return Err("復元通知台帳の版が一致しません".into());
        }
        Ok(Durable {
            writer,
            state,
            text,
        })
    })
    .transpose()
}

#[derive(Clone)]
struct Entry {
    meeting: String,
    scope: String,
    writer: String,
    state: RestoredInputState,
    text: String,
    touched: Instant,
}
#[derive(Default)]
pub(super) struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Storage {
    pub(crate) fn evict_restored_input_cache(&self, meeting: Option<&str>) -> Result<(), String> {
        let mut registry = self.restored_inputs.lock().map_err(|_| "復元通知のキャッシュを解放できません")?;
        registry.entries.retain(|_, entry| meeting.is_some_and(|id| entry.meeting != id));
        Ok(())
    }

    fn verify_restored_input_storage(&self) -> Result<(), String> {
        self.verify_closed_input_storage()?;
        if self.maintenance.load(std::sync::atomic::Ordering::Acquire) {
            return Err("データ操作中は復元通知を保存できません".into());
        }
        Ok(())
    }

    fn restored_input_operation<T>(
        &self,
        meeting: &str,
        owner: &str,
        lease: &NoticeStorageLease,
        action: impl FnOnce(
            &RestoredInputState,
            &PortableInputSnapshot,
        ) -> Result<(RestoredInputState, T), String>,
    ) -> Result<Option<T>, String> {
        lease.validate_active()?;
        self.verify_restored_input_storage()?;
        let mut registry = self
            .restored_inputs
            .lock()
            .map_err(|_| "復元通知の共有所有を取得できません")?;
        let cached = registry.entries.get(owner).cloned();
        let mut invalidate = false;
        let result = (|| {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            // Admission may change while waiting for either mutex. Do not
            // initialize or mutate a destination ledger during maintenance.
            lease.validate_active()?;
            self.verify_restored_input_storage()?;
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let scope = Scope::read(&tx, meeting, owner).map_err(|e| {
                invalidate = true;
                e
            })?;
            let prior = durable(&tx, meeting, owner, &scope.source).map_err(|e| {
                invalidate = true;
                e
            })?;
            let mut entry = if let Some(entry) = cached {
                if entry.meeting != meeting
                    || entry.scope != scope.identity
                    || prior.as_ref().is_none_or(|p| {
                        p.writer != entry.writer
                            || p.text != entry.text
                            || p.state.revision() != entry.state.revision()
                    })
                {
                    invalidate = true;
                    return Err("復元通知の書込み所有または保存版が変更されました".into());
                }
                entry
            } else {
                let state = match &prior {
                    Some(old) => Some(old.state.fenced(&scope.source, owner)?),
                    None => match &scope.transfer {
                        Some(transfer) => Some(transfer.state.fenced(&scope.source, owner)?),
                        None => RestoredInputState::initial(&scope.source, owner)?,
                    },
                };
                let Some(state) = state else {
                    lease.validate_active()?;
                    self.verify_restored_input_storage()?;
                    return Ok(None);
                };
                Entry {
                    meeting: meeting.into(),
                    scope: scope.identity,
                    writer: Uuid::new_v4().to_string(),
                    text: String::new(),
                    state,
                    touched: Instant::now(),
                }
            };
            let (candidate, output) = action(&entry.state, &scope.source)?;
            let text =
                String::from_utf8(candidate.encode(&scope.source)?).map_err(|e| e.to_string())?;
            lease.validate_active()?;
            self.verify_restored_input_storage()?;
            if prior
                .as_ref()
                .is_none_or(|p| p.writer != entry.writer || p.text != text)
            {
                // Guard total allocation without allocating any other row's JSON.
                let (count, bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(CAST(state_json AS BLOB))),0) FROM recording_input_delivery",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
                let previous = prior.as_ref().map_or(0, |p| p.text.len() as i64);
                if count + i64::from(prior.is_none()) > 10_000
                    || bytes - previous + text.len() as i64 > 128 * 1024 * 1024
                {
                    return Err("復元通知台帳の件数または総容量が安全上限を超えました".into());
                }
                let written = if let Some(prior) = &prior {
                    tx.execute("UPDATE recording_input_delivery SET writer_id=?1,revision=?2,state_json=?3 WHERE session_id=?4 AND meeting_id=?5 AND writer_id=?6 AND revision=?7",
                        params![entry.writer,candidate.revision() as i64,text,owner,meeting,prior.writer,prior.state.revision() as i64])
                } else {
                    tx.execute("INSERT INTO recording_input_delivery(session_id,meeting_id,writer_id,revision,state_json) VALUES(?1,?2,?3,?4,?5)",
                        params![owner,meeting,entry.writer,candidate.revision() as i64,text])
                }.map_err(|e|{invalidate=true;e.to_string()})?;
                if written != 1 {
                    invalidate = true;
                    return Err("復元通知台帳の保存所有が失われました".into());
                }
            }
            lease.validate_active()?;
            self.verify_restored_input_storage()?;
            tx.commit().map_err(|e| {
                invalidate = true;
                e.to_string()
            })?;
            lease.validate_active().map_err(|e| {
                invalidate = true;
                e
            })?;
            self.verify_restored_input_storage().map_err(|e| {
                invalidate = true;
                e
            })?;
            entry.state = candidate;
            entry.text = text;
            entry.touched = Instant::now();
            if !registry.entries.contains_key(owner) && registry.entries.len() >= CACHE_LIMIT {
                if let Some(key) = registry
                    .entries
                    .iter()
                    .min_by_key(|(_, v)| v.touched)
                    .map(|(k, _)| k.clone())
                {
                    registry.entries.remove(&key);
                }
            }
            registry.entries.insert(owner.into(), entry);
            Ok(Some(output))
        })();
        if invalidate {
            registry.entries.remove(owner);
        }
        result
    }

    pub(crate) fn restored_input_page(
        &self,
        meeting: &str,
        owner: &str,
        lease: &NoticeStorageLease,
        visible: &impl Fn() -> bool,
    ) -> Result<Option<ArchivedBatch>, String> {
        let started = Instant::now();
        let mut batch = self.restored_input_operation(meeting, owner, lease, |state, source| {
            state.polled(source, visible())
        })?;
        if let Some(batch) = &mut batch {
            batch.account_wait(started.elapsed());
        }
        Ok(batch)
    }
    pub(crate) fn restored_input_acknowledge(
        &self,
        meeting: &str,
        owner: &str,
        lease: &NoticeStorageLease,
        id: &str,
        recipient: &str,
        generation: u64,
        action: Action,
        receive_id: Option<&str>,
        visible: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        let started = Instant::now();
        let receipt = self
            .restored_input_operation(meeting, owner, lease, |state, source| {
                // Renew/expire the queue before acting, just like native archives.
                let (state, _) = state.polled(source, visible())?;
                state.acted(
                    source,
                    id,
                    recipient,
                    generation,
                    action,
                    receive_id,
                    visible(),
                )
            })?
            .ok_or("この復元録音には保存後通知がありません")?;
        if !visible() {
            return Err("復元通知の応答前にメイン画面が非表示になりました".into());
        }
        let mut receipt = receipt;
        receipt.account_wait(started.elapsed());
        Ok(receipt)
    }
}

pub(in crate::storage) fn capture_transfer(
    db: &Connection,
    meeting: &str,
    owner: &str,
    source: &PortableInputSnapshot,
) -> Result<Option<String>, String> {
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('recording_sessions') WHERE name='input_delivery_transfer_json')",[],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        return Ok(None);
    }
    let scope = Scope::read(db, meeting, owner)?;
    if scope.source.encode()? != source.encode()? {
        return Err("復元通知の元記録がバックアップ対象と一致しません".into());
    }
    match durable(db, meeting, owner, source)? {
        Some(current) => Transfer {
            schema_version: 1,
            meeting_id: meeting.into(),
            session_id: owner.into(),
            state: current.state,
        }
        .encode(source)
        .map(Some),
        None => scope.transfer.map(|p| p.encode(source)).transpose(),
    }
}

pub(in crate::storage) fn validate_database(
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('recording_sessions') WHERE name='input_delivery_transfer_json')",[],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {return Ok(());}
    let mut query=db.prepare("SELECT meeting_id,id FROM recording_sessions WHERE input_delivery_transfer_json IS NOT NULL OR id IN (SELECT session_id FROM recording_input_delivery) ORDER BY id LIMIT 10001").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    let mut count = 0;
    let mut total = 0_usize;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if cancelled() {
            return Err("処理をキャンセルしました".into());
        }
        count += 1;
        if count > 10_000 {
            return Err("復元通知の件数が安全上限を超えました".into());
        }
        let meeting: String = row.get(0).map_err(|e| e.to_string())?;
        let owner: String = row.get(1).map_err(|e| e.to_string())?;
        let scope = Scope::read(db, &meeting, &owner)?;
        if let Some(p) = scope.transfer {
            total += p.encode(&scope.source)?.len();
        }
        if let Some(p) = durable(db, &meeting, &owner, &scope.source)? {
            total += p.text.len();
        }
        if total > 128 * 1024 * 1024 {
            return Err("復元通知の総容量が安全上限を超えました".into());
        }
    }
    let orphan:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM recording_input_delivery d LEFT JOIN recording_sessions r ON r.id=d.session_id WHERE r.id IS NULL OR r.meeting_id IS NOT d.meeting_id OR r.input_history_snapshot_json IS NULL)",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if orphan {
        return Err("復元通知台帳の録音所属がありません".into());
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) fn remove_schema_for_test(db: &Connection) {
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=31)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM minutes_markdown", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch(
            "DROP TABLE minutes_markdown; DELETE FROM schema_migrations WHERE version=31;",
        )
        .unwrap();
    }
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=30)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(
            db.query_row("SELECT count(*) FROM recording_input_delivery", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        assert_eq!(db.query_row("SELECT count(*) FROM recording_sessions WHERE input_delivery_transfer_json IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        db.execute_batch("DROP TABLE recording_input_delivery; DROP TRIGGER recording_input_transfer_owner; DROP TRIGGER recording_input_transfer_binding; DROP TRIGGER recording_input_transfer_fixed; ALTER TABLE recording_sessions DROP COLUMN input_delivery_transfer_json; DELETE FROM schema_migrations WHERE version=30;").unwrap();
    }
}
