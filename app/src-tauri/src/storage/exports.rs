use super::*;
use chrono::{Local, TimeZone};
use rusqlite::{Transaction, TransactionBehavior};
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ExportRequest {
    pub operation_id: String,
    pub meeting_id: String,
    pub expected_minutes_id: String,
    pub expected_content_hash: String,
    pub expected_meeting_lock_version: i64,
    pub expected_draft: Option<DraftStamp>,
}
impl ExportRequest {
    fn hash(&self) -> Result<String, String> {
        Uuid::parse_str(&self.operation_id).map_err(|_| "書き出し操作IDが不正です")?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|e| e.to_string())?)
        ))
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportRecord {
    pub id: String,
    pub meeting_id: String,
    pub minutes_revision_id: Option<String>,
    pub exported_content_hash: String,
    pub output_content_hash: String,
    pub output_size_bytes: i64,
    pub destination_path: String,
    pub state: String,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub exported_at: Option<i64>,
}
pub(crate) struct PreparedExport {
    pub request: ExportRequest,
    pub filename: String,
    pub markdown: String,
    pub content_hash: String,
    pub output_hash: String,
}
const COLUMNS: &str = "id,meeting_id,minutes_revision_id,exported_content_hash,output_content_hash,output_size_bytes,destination_path,state,error_message,created_at,exported_at";
fn record(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExportRecord> {
    Ok(ExportRecord {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        minutes_revision_id: r.get(2)?,
        exported_content_hash: r.get(3)?,
        output_content_hash: r.get(4)?,
        output_size_bytes: r.get(5)?,
        destination_path: r.get(6)?,
        state: r.get(7)?,
        error_message: r.get(8)?,
        created_at: r.get(9)?,
        exported_at: r.get(10)?,
    })
}
fn checked_source(
    tx: &Transaction<'_>,
    request: &ExportRequest,
) -> Result<
    (
        String,
        i64,
        crate::minutes_document::MinutesContent,
        MinutesInput,
    ),
    String,
> {
    let (title, created, lock, current): (String, i64, i64, Option<String>) = tx
        .query_row(
            "SELECT title,created_at,lock_version,current_minutes_id FROM meetings WHERE id=?1",
            params![request.meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("会議がありません")?;
    if lock != request.expected_meeting_lock_version
        || current.as_deref() != Some(&request.expected_minutes_id)
    {
        return Err("書き出し前に議事録の現在版が変更されました。確認し直してください".into());
    }
    let view = super::minutes::read_content(tx, &request.meeting_id, &request.expected_minutes_id)?
        .ok_or("現在の議事録がありません")?;
    if view.content_hash != request.expected_content_hash {
        return Err("書き出し元の議事録内容が変更されています".into());
    }
    let draft = super::minutes_drafts::load(tx, &request.meeting_id, None)?;
    let stamp = draft.as_ref().map(|d| DraftStamp {
        id: d.id.clone(),
        lock_version: d.lock_version,
    });
    if stamp != request.expected_draft
        || draft.as_ref().is_some_and(|d| d.base_minutes_id != view.id)
    {
        return Err(
            "書き出し前に議事録の編集が変更されました。保存して確認し直してください".into(),
        );
    }
    let input = super::minutes_input::read_snapshot(
        tx,
        &request.meeting_id,
        &view.input_analysis_id,
        &view.input_content_hash,
    )?;
    Ok((
        title,
        created,
        draft.map(|d| d.payload).unwrap_or(view.content),
        input,
    ))
}
pub(super) fn safe_filename(title: &str, created: i64) -> Result<String, String> {
    let date = Local
        .timestamp_millis_opt(created)
        .single()
        .ok_or("会議日時が不正です")?
        .format("%Y-%m-%d");
    let mut title: String = title
        .chars()
        .take(80)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    while title.len() > 200 {
        title.pop();
    }
    let title = title.trim().trim_matches('.');
    Ok(format!(
        "{date}_{}_議事録.md",
        if title.is_empty() { "会議" } else { title }
    ))
}
impl Storage {
    pub(crate) fn existing_export(
        &self,
        request: &ExportRequest,
    ) -> Result<Option<ExportRecord>, String> {
        let hash = request.hash()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let row: Option<(ExportRecord, String)> = db
            .query_row(
                &format!("SELECT {COLUMNS},request_hash FROM export_records WHERE id=?1"),
                params![request.operation_id],
                |r| Ok((record(r)?, r.get(11)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((r, h)) if h == hash => Ok(Some(r)),
            Some(_) => Err("書き出し操作IDを別の要求には使えません".into()),
            None => Ok(None),
        }
    }
    pub(crate) fn prepare_export(&self, request: ExportRequest) -> Result<PreparedExport, String> {
        request.hash()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (title, created, document, input) = checked_source(&tx, &request)?;
        let text = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
        let content_hash = format!("{:x}", Sha256::digest(&text));
        let markdown = document.markdown(&input)?;
        if markdown.len() > 16 * 1024 * 1024 {
            return Err("書き出すMarkdownが大きすぎます".into());
        }
        let output_hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
        Ok(PreparedExport {
            request,
            filename: safe_filename(&title, created)?,
            markdown,
            content_hash,
            output_hash,
        })
    }
    pub(crate) fn exports(&self, meeting: &str) -> Result<Vec<ExportRecord>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut q=db.prepare(&format!("SELECT {COLUMNS} FROM export_records WHERE meeting_id=?1 ORDER BY created_at DESC,id DESC LIMIT 50")).map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![meeting], record)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub(crate) fn write_export(
        &self,
        prepared: PreparedExport,
        path: &Path,
    ) -> Result<ExportRecord, String> {
        self.write_export_using(prepared, path, |destination, bytes| {
            destination.publish(bytes)
        })
    }
    fn write_export_using(
        &self,
        prepared: PreparedExport,
        path: &Path,
        publish: impl FnOnce(
            &crate::exports::Destination,
            &[u8],
        ) -> Result<(), crate::exports::PublishError>,
    ) -> Result<ExportRecord, String> {
        if let Some(record) = self.existing_export(&prepared.request)? {
            return Ok(record);
        }
        if !path.is_absolute() {
            return Err("保存先には絶対パスが必要です".into());
        }
        // A confirmed destination that cannot be used is still an export failure.
        // Keep its result without ever passing an unsafe path to the publisher.
        let destination = crate::exports::Destination::selected(&self.root, path);
        let record_path = destination
            .as_ref()
            .map(|d| d.path.as_path())
            .unwrap_or(path);
        let record_path = record_path.to_str().ok_or("保存先の文字列が不正です")?;
        let request = &prepared.request;
        {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let (_, _, document, input) = checked_source(&tx, request)?;
            let current_hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&document).map_err(|e| e.to_string())?)
            );
            if current_hash != prepared.content_hash
                || document.markdown(&input)? != prepared.markdown
                || format!("{:x}", Sha256::digest(prepared.markdown.as_bytes()))
                    != prepared.output_hash
            {
                return Err("保存ダイアログ表示中に議事録内容が変わりました".into());
            }
            let now = now_ms()?;
            tx.execute("INSERT INTO export_records(id,meeting_id,minutes_revision_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,source_draft_id,source_draft_lock_version,format,destination_path,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'markdown',?10,'writing',?11,?11)",params![request.operation_id,request.meeting_id,request.expected_minutes_id,request.hash()?,prepared.content_hash,prepared.output_hash,prepared.markdown.len() as i64,request.expected_draft.as_ref().map(|d|&d.id),request.expected_draft.as_ref().map(|d|d.lock_version),record_path,now]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        let result = match destination {
            Ok(destination) => publish(&destination, prepared.markdown.as_bytes()),
            Err(message) => Err(crate::exports::PublishError {
                published: false,
                message,
            }),
        };
        let (state, error) = match result {
            Ok(()) => ("succeeded", None),
            Err(e) => (
                if e.published { "interrupted" } else { "failed" },
                Some(e.message),
            ),
        };
        let now = now_ms()?;
        self.connection.lock().map_err(|_|"DBロックを取得できません")?.execute("UPDATE export_records SET state=?1,error_message=?2,exported_at=?3,updated_at=?4 WHERE id=?5 AND state='writing'",params![state,error,if state=="succeeded"{Some(now)}else{None},now,request.operation_id]).map_err(|e|format!("外部ファイルの書き出し結果をDBへ確定できませんでした。再実行で結果を確認してください: {e}"))?;
        self.existing_export(request)?
            .ok_or_else(|| "書き出し履歴がありません".into())
    }
}

#[cfg(test)]
#[path = "exports_tests.rs"]
mod tests;
