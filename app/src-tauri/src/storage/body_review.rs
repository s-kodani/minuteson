use super::Storage;
use mon_contracts::{MeetingBodyPart, MeetingBodyReview};
use rusqlite::{params, OptionalExtension};

const MAX_REVIEW_BYTES: usize = super::minutes_markdown::MAX_BYTES;
const SQL_MAX_REVIEW_BYTES: i64 = MAX_REVIEW_BYTES as i64;

impl Storage {
    pub(crate) fn meeting_body_for_review(
        &self,
        meeting: &str,
        part: MeetingBodyPart,
    ) -> Result<Option<MeetingBodyReview>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        match part {
            MeetingBodyPart::Minutes => {
                // Return only the readable text. Save stamps, operation IDs and
                // other publication evidence never leave this review path.
                let text = db
                    .query_row(
                        "SELECT CASE WHEN length(CAST(markdown AS BLOB)) <= ?2 THEN markdown END
                     FROM minutes_markdown WHERE meeting_id=?1",
                        params![meeting, SQL_MAX_REVIEW_BYTES],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?
                    .flatten();
                Ok(text
                    .filter(|text| !text.contains('\0'))
                    .map(|text| MeetingBodyReview {
                        text,
                        complete: true,
                    }))
            }
            MeetingBodyPart::Transcript => {
                let mut statement = db.prepare(
                    "SELECT CASE WHEN length(CAST(u.effective_text AS BLOB)) <= ?2 THEN u.effective_text END
                     FROM meetings m
                     JOIN analysis_revisions a ON a.id=m.current_analysis_id AND a.meeting_id=m.id
                     JOIN utterances u ON u.analysis_revision_id=a.id
                     WHERE m.id=?1 AND a.publication_state='committed' ORDER BY u.sequence,u.id",
                ).map_err(|error| error.to_string())?;
                let mut rows = statement
                    .query(params![meeting, SQL_MAX_REVIEW_BYTES])
                    .map_err(|error| error.to_string())?;
                let mut text = String::new();
                let mut count = 0;
                let mut complete = true;
                loop {
                    let row = match rows.next() {
                        Ok(Some(row)) => row,
                        Ok(None) => break,
                        Err(error) if count == 0 => return Err(error.to_string()),
                        Err(_) => {
                            complete = false;
                            break;
                        }
                    };
                    let Ok(Some(line)) = row.get::<_, Option<String>>(0) else {
                        complete = false;
                        continue;
                    };
                    if line.contains('\0') {
                        complete = false;
                        continue;
                    }
                    if text.len() + line.len() + usize::from(count > 0) > MAX_REVIEW_BYTES {
                        complete = false;
                        break;
                    }
                    if count > 0 {
                        text.push('\n');
                    }
                    text.push_str(&line);
                    count += 1;
                }
                Ok((count > 0).then_some(MeetingBodyReview { text, complete }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::minutes_markdown::SaveMarkdownRequest;

    #[test]
    fn damaged_minutes_stamp_allows_only_review_of_the_existing_text() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("確認用").unwrap();
        let request = SaveMarkdownRequest {
            meeting_id: meeting.id.clone(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected: None,
            markdown: "# 元の議事録".to_owned(),
        };
        storage.save_minutes_markdown(request.clone()).unwrap();
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE minutes_markdown SET content_hash=?1 WHERE meeting_id=?2",
                params!["0".repeat(64), &meeting.id],
            )
            .unwrap();

        assert!(storage.get_minutes_markdown(&meeting.id).is_err());
        assert_eq!(
            storage
                .meeting_body_for_review(&meeting.id, MeetingBodyPart::Minutes)
                .unwrap(),
            Some(MeetingBodyReview {
                text: request.markdown.clone(),
                complete: true
            })
        );
        assert!(storage.save_minutes_markdown(request).is_err());
    }
}
