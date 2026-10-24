//! Finite, checkpointed recovery. A rejected engine result is evidence, never
//! transcript text. Only individually checked leaves contribute utterances.
use super::{guard, wave, whisper};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const RATE: u64 = 16_000;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Range {
    first: u64,
    end: u64,
    depth: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Leaf {
    range: Range,
    window_first: u64,
    window_end: u64,
    output: Value,
    review: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct State {
    frames: u64,
    core_first: u64,
    core_end: u64,
    attempts: u32,
    remaining: Vec<Range>,
    leaves: Vec<Leaf>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ReviewRange {
    pub track_id: String,
    pub start_us: i64,
    pub end_us: i64,
    pub reason: String,
}

fn micros(frame: u64) -> i64 {
    (frame * 1_000_000 / RATE) as i64
}
impl State {
    pub(super) fn rebind_models(&mut self, known: &[String], model: &Path) -> Result<(), String> {
        for leaf in &mut self.leaves {
            let name = leaf.output["params"]["model"]
                .as_str()
                .ok_or("再認識モデルがありません")?;
            if !known.iter().any(|n| n == name) {
                return Err("再認識モデルの固定記録が一致しません".into());
            }
            leaf.output["params"]["model"] = json!(model);
        }
        Ok(())
    }
    pub(super) fn validate_input(
        &self,
        first: u64,
        end: u64,
        duration: i64,
        model: &Path,
    ) -> Result<(), String> {
        if (micros(self.frames) - duration).abs() > 63 {
            return Err("再認識の音声長が一致しません".into());
        }
        self.check(first, end.min(self.frames), self.frames, model)
            .map(|_| ())
    }

    pub(super) fn check(
        &self,
        core_first: u64,
        core_end: u64,
        frames: u64,
        model: &Path,
    ) -> Result<Vec<whisper::Segment>, String> {
        if (self.frames, self.core_first, self.core_end) != (frames, core_first, core_end)
            || frames > 124 * RATE
            || core_first >= core_end
            || core_end > frames
            || self.attempts > 30
            || self.remaining.len() + self.leaves.len() > 16
        {
            return Err("再認識の入力範囲・回数が不正です".into());
        }
        let mut ranges: Vec<_> = self
            .remaining
            .iter()
            .chain(self.leaves.iter().map(|l| &l.range))
            .collect();
        ranges.sort_by_key(|r| r.first);
        let mut end = core_first;
        for r in ranges {
            if r.first != end || r.end <= r.first || r.end > core_end || !(1..=4).contains(&r.depth)
            {
                return Err("再認識の範囲に重複・欠落があります".into());
            }
            let mut possible = vec![(core_first, core_end)];
            if core_end - core_first > 8 * RATE {
                for _ in 0..r.depth {
                    possible = possible
                        .into_iter()
                        .flat_map(|(a, b)| [(a, (a + b) / 2), ((a + b) / 2, b)])
                        .collect();
                }
            }
            if !possible.contains(&(r.first, r.end)) {
                return Err("再認識の分割境界が固定手順と一致しません".into());
            }
            end = r.end;
        }
        if end != core_end {
            return Err("再認識の範囲が完了していません".into());
        }
        let mut segments = vec![];
        for leaf in &self.leaves {
            let r = &leaf.range;
            if leaf.window_first != r.first.saturating_sub(RATE)
                || leaf.window_end != (r.end + RATE).min(frames)
            {
                return Err("再認識の文脈範囲が不正です".into());
            }
            let parsed = whisper::parse_output(
                &serde_json::to_vec(&leaf.output).map_err(|e| e.to_string())?,
                model,
                micros(leaf.window_end - leaf.window_first),
            )?;
            let bad = !guard::reasons(&parsed, None).is_empty();
            if bad != leaf.review || (bad && r.depth < 4 && r.end - r.first > 8 * RATE) {
                return Err("再認識の採用判定が不正です".into());
            }
            if !bad {
                for mut s in parsed {
                    let midpoint = (s.start_us + s.end_us) / 2 + micros(leaf.window_first);
                    if midpoint < micros(r.first) || midpoint >= micros(r.end) {
                        continue;
                    }
                    s.start_us = (s.start_us + micros(leaf.window_first)).max(micros(r.first));
                    s.end_us = (s.end_us + micros(leaf.window_first)).min(micros(r.end));
                    if s.end_us > s.start_us {
                        segments.push(s);
                    }
                }
            }
        }
        segments.sort_by_key(|s| (s.start_us, s.end_us));
        Ok(segments)
    }
    pub(super) fn review(&self) -> Vec<(i64, i64)> {
        self.leaves
            .iter()
            .filter(|l| l.review)
            .map(|l| (micros(l.range.first), micros(l.range.end)))
            .collect()
    }
}

pub(super) fn run(
    config: &whisper::WhisperConfig,
    logical_model: &Path,
    directory: &Path,
    core_first: u64,
    core_end: u64,
    state: Option<State>,
    cancelled: &impl Fn() -> bool,
    save: &mut impl FnMut(&State) -> Result<(), String>,
) -> Result<Option<PathBuf>, String> {
    run_with(
        config,
        logical_model,
        directory,
        core_first,
        core_end,
        state,
        cancelled,
        save,
        &mut |config, directory, duration, _| {
            whisper::run_unit_with_suppression(
                config,
                directory,
                duration,
                Duration::from_secs(1800),
                true,
                cancelled,
            )
        },
    )
}
fn run_with(
    config: &whisper::WhisperConfig,
    logical_model: &Path,
    directory: &Path,
    core_first: u64,
    core_end: u64,
    mut state: Option<State>,
    cancelled: &impl Fn() -> bool,
    save: &mut impl FnMut(&State) -> Result<(), String>,
    recognize: &mut impl FnMut(
        &whisper::WhisperConfig,
        &Path,
        i64,
        &dyn Fn() -> bool,
    ) -> Result<Option<Vec<whisper::Segment>>, String>,
) -> Result<Option<PathBuf>, String> {
    let mut wav = vec![];
    fs::File::open(directory.join("input.wav"))
        .map_err(|e| e.to_string())?
        .take(44 + 124 * RATE * 2 + 1)
        .read_to_end(&mut wav)
        .map_err(|e| e.to_string())?;
    if wav.len() < 44 || (wav.len() - 44) % 2 != 0 {
        return Err("再認識用PCMが不正です".into());
    }
    let frames = (wav.len() as u64 - 44) / 2;
    if frames > 124 * RATE || wav[..44] != wave::wav_header(frames as u32) {
        return Err("再認識用WAVが固定形式と一致しません".into());
    }
    let core_end = core_end.min(frames);
    if state.is_none() {
        let mut remaining = vec![];
        if core_end - core_first <= 8 * RATE {
            remaining.push(Range {
                first: core_first,
                end: core_end,
                depth: 4,
            });
        } else {
            let mid = (core_first + core_end) / 2;
            remaining.push(Range {
                first: mid,
                end: core_end,
                depth: 1,
            });
            remaining.push(Range {
                first: core_first,
                end: mid,
                depth: 1,
            });
        }
        state = Some(State {
            frames,
            core_first,
            core_end,
            attempts: 0,
            remaining,
            leaves: vec![],
        });
    }
    let mut state = state.unwrap();
    state.check(core_first, core_end, frames, logical_model)?;
    while let Some(range) = state.remaining.last().cloned() {
        if cancelled() {
            return Ok(None);
        }
        if state.attempts >= 30 {
            return Err("再認識の実行回数が上限を超えました".into());
        }
        let first = range.first.saturating_sub(RATE);
        let end = (range.end + RATE).min(frames);
        let child = directory.join(format!(
            "recovery-{}-{}",
            state.attempts,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&child).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(child.join("input.wav"))
            .map_err(|e| e.to_string())?;
        file.write_all(&wave::wav_header((end - first) as u32))
            .and_then(|_| file.write_all(&wav[44 + first as usize * 2..44 + end as usize * 2]))
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        let mut plain = config.clone();
        plain.prompt.clear();
        if range.depth == 4 || range.end - range.first <= 8 * RATE {
            plain.vad_options = Some(super::vad::VadOptions {
                enabled: false,
                ..plain.vad_options.unwrap_or_default()
            });
        }
        let Some(segments) = recognize(&plain, &child, micros(end - first), cancelled)? else {
            return Ok(None);
        };
        let bad = !guard::reasons(&segments, None).is_empty();
        state.remaining.pop();
        state.attempts += 1;
        if bad && range.depth < 4 && range.end - range.first > 8 * RATE {
            let mid = (range.first + range.end) / 2;
            state.remaining.push(Range {
                first: mid,
                end: range.end,
                depth: range.depth + 1,
            });
            state.remaining.push(Range {
                first: range.first,
                end: mid,
                depth: range.depth + 1,
            });
        } else {
            let bytes = fs::read(child.join("result.json")).map_err(|e| e.to_string())?;
            let bytes = whisper::logical_output(
                &bytes,
                &config.model.path,
                logical_model,
                micros(end - first),
            )?;
            state.leaves.push(Leaf {
                range,
                window_first: first,
                window_end: end,
                output: serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
                review: bad,
            });
        }
        state.check(core_first, core_end, frames, logical_model)?;
        save(&state)?;
    }
    let segments = state.check(core_first, core_end, frames, logical_model)?;
    let result = json!({"model":{"multilingual":true},"params":{"model":logical_model,"language":"ja","translate":false},"transcription":segments.iter().filter(|s|s.end_us/1000>s.start_us/1000).map(|s|json!({"offsets":{"from":s.start_us/1000,"to":s.end_us/1000},"text":s.text})).collect::<Vec<_>>(),"recovery":state});
    let path = directory.join("recovery-result.json");
    crate::restored_processing::write_new(
        &path,
        &serde_json::to_vec(&result).map_err(|e| e.to_string())?,
    )?;
    Ok(Some(path))
}

pub(super) fn checked(
    bytes: &[u8],
    model: &Path,
    duration: i64,
    core_first: u64,
    core_end: u64,
) -> Result<(Vec<whisper::Segment>, State), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let state: State = serde_json::from_value(
        value
            .get("recovery")
            .ok_or("再認識の記録がありません")?
            .clone(),
    )
    .map_err(|e| e.to_string())?;
    if !state.remaining.is_empty() || (micros(state.frames) - duration).abs() > 63 {
        return Err("再認識の完了記録が不正です".into());
    }
    let expected = state.check(core_first, core_end.min(state.frames), state.frames, model)?;
    let parsed = whisper::parse_output(bytes, model, duration)?;
    // CLI wire precision is milliseconds. Compare the canonical wire result.
    let expected: Vec<_> = expected
        .into_iter()
        .map(|mut s| {
            s.start_us = s.start_us / 1000 * 1000;
            s.end_us = s.end_us / 1000 * 1000;
            s
        })
        .filter(|s| s.end_us > s.start_us)
        .collect();
    if expected != parsed {
        return Err("再認識の採用結果が子区間と一致しません".into());
    }
    Ok((parsed, state))
}

pub(super) fn rebind_json(value: &mut Value, known: &[String], model: &Path) -> Result<(), String> {
    if let Some(raw) = value.get("recovery") {
        let mut state: State = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        state.rebind_models(known, model)?;
        value["recovery"] = serde_json::to_value(state).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
