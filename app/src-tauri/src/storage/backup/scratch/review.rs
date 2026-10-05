//! Immutable explicit keep decisions. They do not claim cleanup or publication.
use super::super::recovery::{Evidence, Item, Source};
use super::*;

pub(super) const SCHEMA: &str = "
CREATE TABLE recovery_decisions(source TEXT NOT NULL CHECK(source IN ('scratch','restore_addition')),operation_id TEXT NOT NULL CHECK(length(operation_id)=36),intent_hash TEXT NOT NULL CHECK(length(intent_hash)=64),receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json) AND length(CAST(receipt_json AS BLOB))<=4096),receipt_hash TEXT NOT NULL CHECK(length(receipt_hash)=64),PRIMARY KEY(source,operation_id));
CREATE TRIGGER recovery_decision_fixed BEFORE UPDATE ON recovery_decisions BEGIN SELECT RAISE(ABORT,'keep decision is immutable'); END;
CREATE TRIGGER recovery_decision_retained BEFORE DELETE ON recovery_decisions BEGIN SELECT RAISE(ABORT,'keep decision is retained'); END;
";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    source: String,
    operation_id: String,
    intent_hash: String,
    observation_hash: String,
    confirmation_id: String,
    consent_at: i64,
}
impl Registry {
    pub(in crate::storage::backup) fn known(root: &Path) -> Result<Option<Arc<Self>>, String> {
        let all = REGISTRIES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        Ok(all.get(root).and_then(Weak::upgrade))
    }
    pub(in crate::storage::backup) fn guard(&self) -> Result<(), String> {
        self.check_root()?;
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        self.check_schema(&db)
    }
    pub(in crate::storage::backup) fn scratch_evidence(
        &self,
        id: &str,
    ) -> Result<Evidence, String> {
        self.check_root()?;
        if !Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id) {
            return Err("一時領域IDが不正です".into());
        }
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        self.check_schema(&db)?;
        let (path, proof, sha, state, error, count): (
            String,
            String,
            String,
            String,
            Option<String>,
            i64,
        ) = db
            .query_row(
                "SELECT path,proof,hash,state,error,entry_count FROM areas WHERE id=?1",
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        if path != format!(".mon-stage-{id}")
            || proof.len() > 4096
            || sha.len() != 64
            || !(1..=LIMIT as i64).contains(&count)
            || error.as_ref().is_some_and(|e| e.len() > 8192)
        {
            return Err("一時領域の記録を安全に表示できません".into());
        }
        let mut intent = Sha256::new();
        intent.update(
            serde_json::to_vec(&(id, &path, proof, sha, &self.root_identity))
                .map_err(|e| e.to_string())?,
        );
        let mut status = Sha256::new();
        status.update(serde_json::to_vec(&(&state, &error, count)).map_err(|e| e.to_string())?);
        let mut q = db.prepare("SELECT path,kind,proof,hash,removed FROM entries WHERE area=?1 ORDER BY path LIMIT 200001").map_err(|e|e.to_string())?;
        let mut rows = q.query(params![id]).map_err(|e| e.to_string())?;
        let mut actual = 0;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            actual += 1;
            let (p, k, j, h, r): (String, String, String, String, i64) = (
                row.get(0).map_err(|e| e.to_string())?,
                row.get(1).map_err(|e| e.to_string())?,
                row.get(2).map_err(|e| e.to_string())?,
                row.get(3).map_err(|e| e.to_string())?,
                row.get(4).map_err(|e| e.to_string())?,
            );
            if actual > LIMIT || p.len() > 1024 || j.len() > 4096 || h.len() != 64 {
                return Err("一時領域の表示対象が安全上限を超えています".into());
            }
            intent.update(serde_json::to_vec(&(p, k, j, h)).map_err(|e| e.to_string())?);
            intent.update(b"\n");
            status.update(r.to_le_bytes());
        }
        let live = stages()
            .lock()
            .map_err(|_| "一時領域所有ロックを取得できません")?
            .values()
            .any(|a| {
                a.area == id
                    && a.registry
                        .as_ref()
                        .and_then(Weak::upgrade)
                        .is_some_and(|r| std::ptr::eq(r.as_ref(), self))
            });
        Ok(Evidence {
            item: Item {
                source: Source::Scratch,
                operation_id: id.into(),
                paths: vec![path],
                registered_paths: actual,
                reason: error.unwrap_or_else(|| "前回の構築コピーを回収できていません".into()),
                retained: false,
            },
            state,
            intent_hash: format!("{:x}", intent.finalize()),
            status_hash: format!("{:x}", status.finalize()),
            live,
        })
    }
    pub(in crate::storage::backup) fn pending_scratch(&self) -> Result<Vec<String>, String> {
        self.guard()?;
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let mut q = db
            .prepare("SELECT id FROM areas WHERE state!='recovered' ORDER BY id LIMIT 2001")
            .map_err(|e| e.to_string())?;
        let ids = q
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        if ids.len() > 2000 {
            return Err("要確認記録が表示上限を超えています。既存ファイルは保持します".into());
        }
        Ok(ids)
    }
    pub(in crate::storage::backup) fn retained(
        &self,
        source: &str,
        id: &str,
        intent: &str,
    ) -> Result<bool, String> {
        self.guard()?;
        let db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let row: Option<(String,String,String)> = db.query_row("SELECT intent_hash,receipt_json,receipt_hash FROM recovery_decisions WHERE source=?1 AND operation_id=?2",params![source,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
        let Some((saved, json, sha)) = row else {
            return Ok(false);
        };
        if saved != intent || json.len() > 4096 || hash(json.as_bytes()) != sha {
            return Err("保持の確認記録が変更されています。対象を保持します".into());
        }
        let d: Decision = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if d.source != source
            || d.operation_id != id
            || d.intent_hash != intent
            || !portable::valid_hash(&d.observation_hash)
            || !Uuid::parse_str(&d.confirmation_id)
                .is_ok_and(|u| u.to_string() == d.confirmation_id)
            || d.consent_at < 0
        {
            return Err("保持の確認記録を検証できません。対象を保持します".into());
        }
        Ok(true)
    }
    pub(in crate::storage::backup) fn keep(
        &self,
        source: &str,
        id: &str,
        intent: &str,
        observation: &str,
        confirmation: &str,
    ) -> Result<(), String> {
        if self.retained(source, id, intent)? {
            return Ok(());
        }
        self.guard()?;
        let decision = Decision {
            source: source.into(),
            operation_id: id.into(),
            intent_hash: intent.into(),
            observation_hash: observation.into(),
            confirmation_id: confirmation.into(),
            consent_at: now_ms()?,
        };
        let json = serde_json::to_string(&decision).map_err(|e| e.to_string())?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| "一時領域台帳ロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO recovery_decisions VALUES(?1,?2,?3,?4,?5)",
            params![source, id, intent, json, hash(json.as_bytes())],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub(in crate::storage::backup) fn recover_selected(&self, id: &str) -> Result<(), String> {
        let evidence = self.scratch_evidence(id)?;
        if evidence.live || self.retained("scratch", id, &evidence.intent_hash)? {
            return Err("利用中または保持して確認済みの領域は回収しません".into());
        }
        self.cleanup(id, &self.root.join(format!(".mon-stage-{id}")))
    }
}
