//! Export the sole human-owned Markdown body, byte for byte. No revision,
//! structured rendering, old-body copy or source transcript is needed.
use super::exports::{record, COLUMNS};
use super::minutes_markdown::{self, MarkdownDocument, MarkdownStamp};
use super::*;
use rusqlite::{Transaction, TransactionBehavior};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MarkdownExportRequest {
    pub operation_id: String,
    pub meeting_id: String,
    pub expected: MarkdownStamp,
}
impl MarkdownExportRequest {
    fn hash(&self) -> Result<String, String> {
        if !minutes_markdown::uuid(&self.operation_id)
            || self.expected.lock_version < 1
            || self.expected.lock_version > 9_007_199_254_740_991
            || self.expected.content_hash.len() != 64
            || !self
                .expected
                .content_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("書き出し操作ID・確認済み保存状態が不正です".into());
        }
        let mut hash = Sha256::new();
        hash.update(b"mon-plain-markdown-export-v1\0");
        hash.update(serde_json::to_vec(self).map_err(|e| e.to_string())?);
        Ok(format!("{:x}", hash.finalize()))
    }
}
pub(crate) struct PreparedMarkdownExport {
    request: MarkdownExportRequest,
    document: MarkdownDocument,
    output_hash: String,
    pub filename: String,
}
fn checked_source(
    tx: &Transaction<'_>,
    request: &MarkdownExportRequest,
) -> Result<(String, i64, MarkdownDocument), String> {
    let (title, created): (String, i64) = tx
        .query_row(
            "SELECT title,created_at FROM meetings WHERE id=?1",
            [&request.meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("会議がありません")?;
    let document =
        minutes_markdown::read(tx, &request.meeting_id)?.ok_or("保存済みの議事録がありません")?;
    if document.stamp() != request.expected {
        return Err("書き出し前に議事録が変更されました。保存状態を確認してください".into());
    }
    Ok((title, created, document))
}
impl Storage {
    pub(crate) fn existing_markdown_export(
        &self,
        request: &MarkdownExportRequest,
    ) -> Result<Option<ExportRecord>, String> {
        let hash = request.hash()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let row: Option<(ExportRecord, String)> = db
            .query_row(
                &format!("SELECT {COLUMNS},request_hash FROM export_records WHERE id=?1"),
                [&request.operation_id],
                |r| Ok((record(r)?, r.get("request_hash")?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((record, saved_hash)) if hash == saved_hash => Ok(Some(record)),
            Some(_) => Err("書き出し操作IDを別の要求には使えません".into()),
            None => Ok(None),
        }
    }
    pub(crate) fn prepare_markdown_export(
        &self,
        request: MarkdownExportRequest,
    ) -> Result<PreparedMarkdownExport, String> {
        request.hash()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (title, created, document) = checked_source(&tx, &request)?;
        let output_hash = format!("{:x}", Sha256::digest(document.markdown.as_bytes()));
        Ok(PreparedMarkdownExport {
            request,
            document,
            output_hash,
            filename: super::exports::safe_filename(&title, created)?,
        })
    }
    pub(crate) fn write_markdown_export(
        &self,
        prepared: PreparedMarkdownExport,
        path: &Path,
    ) -> Result<ExportRecord, String> {
        self.write_markdown_export_using(prepared, path, |destination, bytes| {
            destination.publish(bytes)
        })
    }
    fn write_markdown_export_using(
        &self,
        prepared: PreparedMarkdownExport,
        path: &Path,
        publish: impl FnOnce(
            &crate::exports::Destination,
            &[u8],
        ) -> Result<(), crate::exports::PublishError>,
    ) -> Result<ExportRecord, String> {
        if let Some(record) = self.existing_markdown_export(&prepared.request)? {
            return Ok(record);
        }
        if !path.is_absolute() {
            return Err("保存先には絶対パスが必要です".into());
        }
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
            let (_, _, document) = checked_source(&tx, request)?;
            if document != prepared.document
                || format!("{:x}", Sha256::digest(document.markdown.as_bytes()))
                    != prepared.output_hash
            {
                return Err("保存ダイアログ表示中に議事録内容が変わりました".into());
            }
            let now = now_ms()?;
            tx.execute("INSERT INTO export_records(id,meeting_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,format,destination_path,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'markdown',?7,'writing',?8,?8)",
                params![request.operation_id,request.meeting_id,request.hash()?,document.content_hash,prepared.output_hash,document.markdown.len() as i64,record_path,now]).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        // Release the DB lock before durable external I/O. The admitted snapshot
        // is fixed; subsequent human edits cannot change the bytes being exported.
        let result = match destination {
            Ok(destination) => publish(&destination, prepared.document.markdown.as_bytes()),
            Err(message) => Err(crate::exports::PublishError {
                published: false,
                message,
            }),
        };
        let (state, error) = match result {
            Ok(()) => ("succeeded", None),
            Err(error) => (
                if error.published {
                    "interrupted"
                } else {
                    "failed"
                },
                Some(error.message),
            ),
        };
        let now = now_ms()?;
        self.connection.lock().map_err(|_| "DBロックを取得できません")?.execute(
            "UPDATE export_records SET state=?1,error_message=?2,exported_at=?3,updated_at=?4 WHERE id=?5 AND state='writing'",
            params![state,error,if state=="succeeded" { Some(now) } else { None },now,request.operation_id])
            .map_err(|e| format!("外部ファイルの書き出し結果をDBへ確定できませんでした。再実行で結果を確認してください: {e}"))?;
        self.existing_markdown_export(request)?
            .ok_or_else(|| "書き出し記録がありません".into())
    }
}

#[cfg(test)]
#[path = "markdown_exports_tests.rs"]
mod tests;
