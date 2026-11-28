//! 文字起こしの同一性ハッシュから独立した適用診断。
use super::*;
use crate::transcription::transducer::{MAX_HOTWORD_BYTES, MAX_IGNORED_HOTWORDS};
use mon_contracts::IgnoredHotword;

const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;

fn valid(terms: &[IgnoredHotword]) -> bool {
    terms.len() <= MAX_IGNORED_HOTWORDS
        && terms
            .iter()
            .all(|term| term.term.len() <= MAX_HOTWORD_BYTES)
        && terms.windows(2).all(|pair| pair[0].term < pair[1].term)
}

pub(super) fn read(db: &Connection, revision: &str) -> Result<Vec<IgnoredHotword>, String> {
    let json: Option<String> = db
        .query_row(
            "SELECT ignored_hotwords_json FROM analysis_vocabulary_diagnostics WHERE analysis_revision_id=?1",
            [revision],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    if json.len() > MAX_JSON_BYTES {
        return Err("除外用語の診断記録が大きすぎます".into());
    }
    let terms: Vec<IgnoredHotword> =
        serde_json::from_str(&json).map_err(|error| error.to_string())?;
    if !valid(&terms) {
        return Err("除外用語の診断記録が不正です".into());
    }
    Ok(terms)
}

pub(super) fn insert(
    db: &Connection,
    revision: &str,
    terms: &[IgnoredHotword],
    discard_oversized: bool,
) -> Result<(), String> {
    if terms.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(terms).map_err(|error| error.to_string())?;
    if !valid(terms) || json.len() > MAX_JSON_BYTES {
        let message = format!(
            "除外用語の診断記録が不正または容量超過です（{}件）",
            terms.len()
        );
        if discard_oversized {
            crate::internal_logs::report_error("transcription", &message);
            return Ok(());
        }
        return Err(message);
    }
    db.execute(
        "INSERT INTO analysis_vocabulary_diagnostics(analysis_revision_id,ignored_hotwords_json) VALUES(?1,?2)",
        params![revision, json],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn copy(db: &Connection, from: &str, to: &str) -> Result<(), String> {
    db.execute(
        "INSERT INTO analysis_vocabulary_diagnostics(analysis_revision_id,ignored_hotwords_json)
         SELECT ?1,ignored_hotwords_json FROM analysis_vocabulary_diagnostics WHERE analysis_revision_id=?2",
        params![to, from],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
