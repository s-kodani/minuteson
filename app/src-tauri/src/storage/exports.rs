//! Shared receipt decoding and filenames for plain Markdown exports.
//! Past receipts remain data, not a second structured-minutes writer.
use super::*;
use chrono::{Local, TimeZone};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportRecord {
    pub id: String,
    pub meeting_id: String,
    pub exported_content_hash: String,
    pub output_content_hash: String,
    pub output_size_bytes: i64,
    pub destination_path: String,
    pub state: String,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub exported_at: Option<i64>,
}
pub(super) const COLUMNS: &str = "id,meeting_id,exported_content_hash,output_content_hash,output_size_bytes,destination_path,state,error_message,created_at,exported_at";
pub(super) fn record(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExportRecord> {
    Ok(ExportRecord {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        exported_content_hash: r.get(2)?,
        output_content_hash: r.get(3)?,
        output_size_bytes: r.get(4)?,
        destination_path: r.get(5)?,
        state: r.get(6)?,
        error_message: r.get(7)?,
        created_at: r.get(8)?,
        exported_at: r.get(9)?,
    })
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

#[cfg(test)]
impl Storage {
    pub(super) fn exports(&self, meeting: &str) -> Result<Vec<ExportRecord>, String> {
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
}
#[cfg(test)]
#[path = "exports_tests.rs"]
mod tests;
