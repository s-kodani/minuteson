//! Cheap availability observations. Publication/playback still verify the full
//! audio hash; list and summary reads do not scan long recordings or mutate rows.
use super::*;

pub(in crate::storage) struct Source {
    pub meeting: String,
    asset: String,
    relative: String,
    size: i64,
    hash: String,
}

pub(in crate::storage) struct Candidate {
    pub meeting: Option<String>,
    pub source: Result<Source, String>,
}

pub(in crate::storage) enum Issue {
    Missing,
    Unreadable(String),
}

pub(in crate::storage) enum Observation {
    Available,
    Unavailable(Issue),
    Changed,
}

impl Issue {
    pub(in crate::storage) fn reason(&self) -> &'static str {
        match self {
            Self::Missing => "audio_missing",
            Self::Unreadable(_) => "audio_unreadable",
        }
    }

    pub(in crate::storage) fn message(&self) -> String {
        match self {
            Self::Missing => "保存済みの音声ファイルが見つかりません。文字起こし・議事録は保持しています".into(),
            Self::Unreadable(error) => format!("保存済みの音声ファイルを読み取れません。文字起こし・議事録は保持しています: {error}"),
        }
    }
}

impl Source {
    pub(in crate::storage) fn ready(db: &Connection) -> Result<Vec<Candidate>, String> {
        let unreserved = super::super::meeting_access::unreserved_sql(db, "a.meeting_id")?;
        let mut query = db
            .prepare(&format!(
                "SELECT a.meeting_id,f.id,f.relative_path,f.size_bytes,f.content_hash
             FROM meeting_audio_commits a
             JOIN audio_tracks t ON t.id=a.track_id AND t.meeting_id=a.meeting_id
             JOIN audio_chunks c ON c.track_id=t.id AND c.meeting_id=t.meeting_id
             JOIN file_assets f ON f.id=c.file_asset_id AND f.meeting_id=c.meeting_id
             WHERE a.phase IN ('committed','cleaned') AND t.kind='processed' AND t.format='wav'
             AND t.state='ready' AND c.state='ready' AND f.state='ready'{unreserved}"
            ))
            .map_err(|error| error.to_string())?;
        let rows = query
            .query_map([], |row| {
                let meeting = row.get::<_, String>(0);
                let scope = meeting.as_ref().ok().cloned();
                let source = (|| -> rusqlite::Result<Self> {
                    Ok(Self {
                        meeting: meeting?,
                        asset: row.get(1)?,
                        relative: row.get(2)?,
                        size: row.get(3)?,
                        hash: row.get(4)?,
                    })
                })();
                Ok(Candidate {
                    meeting: scope,
                    source: source.map_err(|error| error.to_string()),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub(in crate::storage) fn current(&self, db: &Connection) -> Result<bool, String> {
        if super::super::meeting_access::reserved(db, &self.meeting)?
            || super::super::deletion::fence::audio_pending(db, &self.meeting)?
        {
            return Ok(false);
        }
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM meeting_audio_commits a
             JOIN audio_tracks t ON t.id=a.track_id AND t.meeting_id=a.meeting_id
             JOIN audio_chunks c ON c.track_id=t.id AND c.meeting_id=t.meeting_id
             JOIN file_assets f ON f.id=c.file_asset_id AND f.meeting_id=c.meeting_id
             WHERE f.id=?1 AND a.meeting_id=?2 AND f.relative_path=?3
             AND f.size_bytes=?4 AND f.content_hash=?5
             AND a.phase IN ('committed','cleaned') AND t.kind='processed' AND t.format='wav'
             AND t.state='ready' AND c.state='ready' AND f.state='ready')",
            params![
                self.asset,
                self.meeting,
                self.relative,
                self.size,
                self.hash
            ],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    }

    pub(in crate::storage) fn inspect(&self, storage: &Storage) -> Result<Observation, String> {
        self.inspect_observed(storage, || {})
    }

    fn inspect_observed(
        &self,
        storage: &Storage,
        observed: impl FnOnce(),
    ) -> Result<Observation, String> {
        // Reserve/delete admission takes this same lock. Keep only one
        // meeting's bounded open/metadata/size check inside the guard; never
        // hash/read audio here or hold the guard across the complete list.
        // Every file and parent handle is dropped before the guard, including
        // on error, so an accepted deletion cannot meet our Windows pins.
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if !self.current(&db)? {
            return Ok(Observation::Changed);
        }
        let result = self.inspect_file(storage, observed);
        if !self.current(&db)? {
            return Ok(Observation::Changed);
        }
        Ok(match result {
            Ok(()) => Observation::Available,
            Err(issue) => Observation::Unavailable(issue),
        })
    }

    fn inspect_file(&self, storage: &Storage, observed: impl FnOnce()) -> Result<(), Issue> {
        let check = || -> Result<_, String> {
            let relative = Path::new(&self.relative);
            if self.size < 0 || !relative.starts_with(Path::new("meetings").join(&self.meeting)) {
                return Err("会議音声の保存記録・管理領域が不正です".into());
            }
            let Some(file) = storage.managed_anchor.readable_file(relative)? else {
                return Ok(false);
            };
            observed();
            if file.metadata().map_err(|error| error.to_string())?.len() != self.size as u64 {
                return Err("会議音声のサイズが保存記録と一致しません".into());
            }
            Ok(true)
        };
        match check() {
            Ok(true) => Ok(()),
            Ok(false) => Err(Issue::Missing),
            Err(error) => Err(Issue::Unreadable(error)),
        }
    }
}

impl Storage {
    pub(in crate::storage) fn require_canonical_audio_available(
        &self,
        meeting: &str,
        plan: &RecordedAudioPlan,
    ) -> Result<(), String> {
        let chunk = &plan.tracks[0].chunks[0];
        let source = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            db.query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2
                AND size_bytes=?3 AND content_hash=?4 AND state='ready'",
                params![chunk.asset_id, meeting, chunk.size, chunk.hash],
                |row| row.get(0),
            )
            .map(|relative| Source {
                meeting: meeting.into(),
                asset: chunk.asset_id.clone(),
                relative,
                size: chunk.size,
                hash: chunk.hash.clone(),
            })
            .map_err(|error| error.to_string())?
        };
        let observation = source.inspect(self)?;
        if matches!(observation, Observation::Changed)
            || self.canonical_audio_plan(meeting)? != *plan
        {
            return Err("確認中に会議音声の保存記録が変更されました".into());
        }
        match observation {
            Observation::Available => Ok(()),
            Observation::Unavailable(issue) => Err(issue.message()),
            Observation::Changed => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests;
