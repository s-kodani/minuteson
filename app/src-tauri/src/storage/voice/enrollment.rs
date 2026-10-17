//! Registration follows saved transcript assignments. Matching retains its
//! original, stricter physical-speaker identity policy in the parent module.
use super::*;

#[cfg(all(test, unix))]
mod tests;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Candidate {
    #[serde(flatten)]
    pub window: Window,
    pub utterance_id: String,
    pub text: String,
}

pub(super) fn candidates(
    tx: &Transaction<'_>,
    view: &AnalysisView,
) -> Result<Vec<Candidate>, String> {
    type Event = (i64, u8, String, i32);
    let utterances: BTreeMap<_, _> = view.utterances.iter().map(|u| (u.id.as_str(), u)).collect();
    let mut tracks: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    let mut interval = |track: String, start, end, kind, key: String| {
        let events = tracks.entry(track).or_default();
        events.push((start, kind, key.clone(), 1));
        events.push((end, kind, key, -1));
    };
    let mut q = tx.prepare("SELECT track_id,start_us,end_us,COALESCE(speaker_id,'') FROM speaker_segments WHERE analysis_revision_id=?1").map_err(|e| e.to_string())?;
    let mut rows = q.query([&view.id]).map_err(|e| e.to_string())?;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > 100_000 {
            return Err("声の登録区間が上限を超えています".into());
        }
        interval(
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            0,
            row.get(3).map_err(|e| e.to_string())?,
        );
    }
    // Use utterance IDs, not speaker IDs, to keep utterance boundaries and
    // overlapping transcript rows even when they name the same person.
    for u in &view.utterances {
        interval(u.track_id.clone(), u.start_us, u.end_us, 1, u.id.clone());
    }
    let mut q = tx.prepare("SELECT c.track_id,c.start_us,c.end_us FROM audio_chunks c JOIN file_assets f ON f.id=c.file_asset_id WHERE c.meeting_id=?1 AND c.state='ready' AND f.state='ready'").map_err(|e| e.to_string())?;
    let mut rows = q.query([&view.meeting_id]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        interval(
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            2,
            String::new(),
        );
    }
    let mut q = tx.prepare("SELECT g.track_id,g.start_us,g.end_us FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id WHERE t.meeting_id=?1 AND g.recovered=0").map_err(|e| e.to_string())?;
    let mut rows = q.query([&view.meeting_id]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        interval(
            row.get(0).map_err(|e| e.to_string())?,
            row.get(1).map_err(|e| e.to_string())?,
            row.get(2).map_err(|e| e.to_string())?,
            3,
            String::new(),
        );
    }
    let mut spans: Vec<(Candidate, String)> = Vec::new();
    for (track, mut events) in tracks {
        events.sort();
        let mut active = [BTreeMap::<String, i32>::new(), BTreeMap::new()];
        let (mut coverage, mut gaps, mut previous) = (0, 0, 0);
        for (time, kind, key, delta) in events {
            if time > previous
                && coverage > 0
                && gaps == 0
                && active[0].len() == 1
                && active[1].len() == 1
            {
                let physical = active[0].keys().next().unwrap();
                let id = active[1].keys().next().unwrap();
                if let Some(u) = utterances.get(id.as_str()).filter(|u| {
                    !physical.is_empty()
                        && u.speaker_ids.len() == 1
                        && view.speakers.iter().any(|s| s.id == u.speaker_ids[0])
                }) {
                    if let Some((last, _)) = spans.last_mut().filter(|(c, source)| {
                        c.window.track_id == track
                            && c.utterance_id == *id
                            && c.window.end_us == previous
                            && source == physical
                    }) {
                        last.window.end_us = time;
                    } else {
                        spans.push((
                            Candidate {
                                window: Window {
                                    track_id: track.clone(),
                                    speaker_id: u.speaker_ids[0].clone(),
                                    start_us: previous,
                                    end_us: time,
                                },
                                utterance_id: id.clone(),
                                text: u.effective_text.clone(),
                            },
                            physical.clone(),
                        ));
                    }
                }
            }
            if kind < 2 {
                let values = &mut active[kind as usize];
                let n = values.get(&key).copied().unwrap_or(0) + delta;
                if n == 0 {
                    values.remove(&key);
                } else {
                    values.insert(key, n);
                }
            } else if kind == 2 {
                coverage += delta;
            } else {
                gaps += delta;
            }
            previous = time;
        }
    }
    let mut result: Vec<_> = spans
        .into_iter()
        .filter_map(|(mut c, _)| {
            let duration = c.window.end_us - c.window.start_us;
            if duration < 1_000_000 {
                return None;
            }
            let length = duration.min(30_000_000);
            c.window.start_us += (duration - length) / 2;
            c.window.end_us = c.window.start_us + length;
            Some(c)
        })
        .collect();
    result.sort_by(|a, b| {
        (a.window.start_us, &a.window.track_id, &a.utterance_id).cmp(&(
            b.window.start_us,
            &b.window.track_id,
            &b.utterance_id,
        ))
    });
    Ok(result)
}

pub(super) fn validate_selection(
    candidates: &[Candidate],
    speaker: &str,
    selected: &[Window],
) -> Result<(), String> {
    if selected.is_empty() || selected.len() > 2 {
        return Err("登録区間を1〜2区間選んでください".into());
    }
    for (index, w) in selected.iter().enumerate() {
        if w.speaker_id != speaker || !candidates.iter().any(|c| c.window == *w) {
            return Err("登録候補が変更されています。区間を選び直してください".into());
        }
        if selected[..index].iter().any(|other| {
            other.track_id == w.track_id && other.start_us < w.end_us && w.start_us < other.end_us
        }) {
            return Err("重複する登録区間は選べません".into());
        }
    }
    Ok(())
}
