//! Immutable, bounded analysis runs on the original meeting-time lattice.
//! Gap row IDs are deliberately absent so additive restore changes only IDs.
use super::*;
use crate::audio_clock::frame_at;
use crate::playback::PlaybackMissingRange;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct KnownInput {
    pub duration_us: i64,
    pub gaps: Vec<Gap>,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Gap {
    pub track_index: usize,
    pub start_us: i64,
    pub end_us: i64,
    pub reason_code: String,
    pub recovered: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Run {
    pub track_index: usize,
    pub start_us: i64,
    pub end_us: i64,
    pub first_frame: u64,
    pub end_frame: u64,
}

impl KnownInput {
    pub(crate) fn validate(&self, source: &RecordedAudioPlan) -> Result<(), String> {
        if self.runs != runs(source, self.duration_us, &self.gaps)? {
            return Err("固定した保存区間と欠損・取得時計が一致しません".into());
        }
        Ok(())
    }
}

fn covers(gaps: &[&Gap], start: i64, end: i64) -> bool {
    let mut position = start;
    for gap in gaps {
        if gap.end_us <= position {
            continue;
        }
        if gap.start_us > position {
            break;
        }
        position = position.max(gap.end_us);
        if position >= end {
            return true;
        }
    }
    position >= end
}

fn runs(source: &RecordedAudioPlan, duration: i64, gaps: &[Gap]) -> Result<Vec<Run>, String> {
    if source.tracks.is_empty()
        || source.tracks.len() > 2
        || duration <= 0
        || duration > 7 * 24 * 3600 * 1_000_000
        || gaps.len() > 2 * crate::audio_clock::MAX_CLOCK_GAPS
    {
        return Err("保存区間の解析構成が範囲外です".into());
    }
    for (index, gap) in gaps.iter().enumerate() {
        let track = source
            .tracks
            .get(gap.track_index)
            .ok_or("欠損の固定トラックがありません")?;
        PlaybackMissingRange::new(
            &track.kind,
            gap.start_us,
            gap.end_us,
            &gap.reason_code,
            duration,
        )?;
        if index > 0 {
            let previous = &gaps[index - 1];
            if (
                previous.track_index,
                previous.start_us,
                previous.end_us,
                &previous.reason_code,
                previous.recovered,
            ) > (
                gap.track_index,
                gap.start_us,
                gap.end_us,
                &gap.reason_code,
                gap.recovered,
            ) {
                return Err("欠損の固定順序が不正です".into());
            }
        }
    }
    if source.tracks.len() == 1 && source.tracks[0].kind == "processed" {
        let t = &source.tracks[0];
        return Ok(vec![Run {
            track_index: 0,
            start_us: 0,
            end_us: t.end_us,
            first_frame: 0,
            end_frame: t.total_frames()?,
        }]);
    }
    let mut result = Vec::new();
    for (track_index, track) in source.tracks.iter().enumerate() {
        if !(1..=8).contains(&track.channels) || track.end_us > duration {
            return Err("保存区間の形式・終端が不正です".into());
        }
        let layout = track.clock_layout()?;
        let track_gaps: Vec<_> = gaps
            .iter()
            .filter(|gap| gap.track_index == track_index)
            .collect();
        if track_gaps.len() > crate::audio_clock::MAX_CLOCK_GAPS {
            return Err("入力の欠損記録が多すぎます".into());
        }
        let missing: Vec<_> = track_gaps
            .into_iter()
            .filter(|gap| !gap.recovered)
            .collect();
        let mut gap_cursor = 0;
        for pair in layout.spans.windows(2) {
            // Spans and missing rows are time ordered. Do not rescan an entire
            // dense gap prefix for each successive observed clock hole.
            while gap_cursor < missing.len() && missing[gap_cursor].end_us <= pair[0].end_us {
                gap_cursor += 1;
            }
            if pair[0].end_us < pair[1].start_us
                && !covers(&missing[gap_cursor..], pair[0].end_us, pair[1].start_us)
            {
                return Err("取得時計の空白に未回復の欠損証拠がありません".into());
            }
        }
        let mut position = track.start_us;
        for gap in missing {
            if gap.end_us <= position || gap.start_us >= track.end_us {
                continue;
            }
            let end = gap.start_us.min(track.end_us);
            if end > position {
                append(
                    &mut result,
                    track_index,
                    position,
                    end,
                    track.sample_rate,
                    true,
                )?;
            }
            position = position.max(gap.end_us).min(track.end_us);
        }
        if position < track.end_us {
            append(
                &mut result,
                track_index,
                position,
                track.end_us,
                track.sample_rate,
                false,
            )?;
        }
        // An entirely unavailable track must not turn into an empty success.
        if !result.iter().any(|run| run.track_index == track_index) {
            return Err("この入力には解析可能な保存音声がありません".into());
        }
    }
    if result.len() > 2 * (crate::audio_clock::MAX_CLOCK_GAPS + 1) {
        return Err("解析可能な保存区間が多すぎます".into());
    }
    Ok(result)
}

fn append(
    result: &mut Vec<Run>,
    track_index: usize,
    start: i64,
    end: i64,
    rate: u32,
    before_gap: bool,
) -> Result<(), String> {
    let first = frame_at(start, rate)?;
    let last = if before_gap {
        (end as u128 * u128::from(rate) / 1_000_000) as u64
    } else {
        frame_at(end, rate)?
    };
    // A gap-adjacent fractional sample is excluded, never fabricated or joined.
    if last > first {
        result.push(Run {
            track_index,
            start_us: start,
            end_us: end,
            first_frame: first,
            end_frame: last,
        });
    }
    Ok(())
}

pub(in crate::storage) fn read(
    db: &Connection,
    meeting: &str,
    source: &RecordedAudioPlan,
) -> Result<KnownInput, String> {
    let duration: Option<i64> = db
        .query_row(
            "SELECT duration_us FROM meetings WHERE id=?1",
            params![meeting],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut gaps = Vec::new();
    for (track_index, track) in source.tracks.iter().enumerate() {
        let mut query=db.prepare("SELECT start_us,end_us,reason_code,recovered FROM audio_gaps WHERE track_id=?1 ORDER BY start_us,end_us,reason_code,recovered,id LIMIT ?2").map_err(|e|e.to_string())?;
        let selected = query
            .query_map(
                params![track.id, (crate::audio_clock::MAX_CLOCK_GAPS + 1) as i64],
                |r| {
                    Ok(Gap {
                        track_index,
                        start_us: r.get(0)?,
                        end_us: r.get(1)?,
                        reason_code: r.get(2)?,
                        recovered: r.get(3)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if selected.len() > crate::audio_clock::MAX_CLOCK_GAPS {
            return Err("入力の欠損記録が多すぎます".into());
        }
        gaps.extend(selected);
    }
    let duration = match duration.filter(|duration| *duration > 0) {
        Some(duration) => duration,
        None if gaps.is_empty() => source
            .tracks
            .iter()
            .map(|track| track.end_us)
            .max()
            .unwrap_or(0),
        None => return Err("欠損を含む音声の保存済み会議時間を確認できません".into()),
    };
    let runs = runs(source, duration, &gaps)?;
    Ok(KnownInput {
        duration_us: duration,
        gaps,
        runs,
    })
}

impl Storage {
    pub(crate) fn prepare_analysis_track(
        &self,
        meeting: &str,
        track: &TrackPlan,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::playback::PreparedAudio, String> {
        let audio = self.prepare_meeting_track_audio_cancellable(
            meeting,
            &track.id,
            track.start_us,
            track.end_us,
            cancelled,
        )?;
        let first = frame_at(track.start_us, track.sample_rate)?;
        let end = frame_at(track.end_us, track.sample_rate)?;
        if audio.sample_rate != track.sample_rate
            || audio.channels != track.channels
            || audio.total_frames() != end - first
        {
            return Err("固定した保存音声の形式・フレーム数が変更されています".into());
        }
        Ok(audio)
    }

    pub(crate) fn prepare_analysis_run(
        &self,
        meeting: &str,
        track: &TrackPlan,
        run: &Run,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::playback::PreparedAudio, String> {
        track
            .clock_layout()?
            .range(run.start_us, run.end_us, track.sample_rate)?;
        let mut audio = self.prepare_analysis_track(meeting, track, cancelled)?;
        audio
            .clock
            .as_mut()
            .ok_or("保存区間の取得時計がありません")?
            .restrict_to_known(run.first_frame, run.end_frame)?;
        audio.start_us = run.start_us;
        audio.end_us = run.end_us;
        Ok(audio)
    }

    pub(crate) fn recorded_analysis_input(
        &self,
        meeting: &str,
    ) -> Result<(RecordedAudioPlan, Option<KnownInput>), String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let source = read_analysis_audio_plan(&tx, meeting, None)?;
        let known = read(&tx, meeting, &source)?;
        Ok((source, Some(known)))
    }
}
