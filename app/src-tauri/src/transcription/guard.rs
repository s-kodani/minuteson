//! Versioned quality gate. Normalization is for comparison only; accepted
//! engine text and timestamps are never edited by this module.
use super::whisper::{self, AsrConfig, Segment};
use crate::storage::VocabularySnapshot;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) enum Reason {
    Repetition,
    ReadingEcho,
    VocabularyEcho,
    TimestampGap,
}

pub(super) fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn repeated(items: &[(String, i64, i64)]) -> bool {
    for first in 0..items.len() {
        for width in 1..=3 {
            if first + width * 3 > items.len() {
                continue;
            }
            // Do not turn nine short acknowledgements into a qualifying
            // three-segment phrase merely by grouping them in threes.
            if (1..width).any(|p| {
                width % p == 0 && (0..width).all(|i| items[first + i].0 == items[first + i % p].0)
            }) {
                continue;
            }
            let chars: usize = items[first..first + width]
                .iter()
                .map(|v| v.0.chars().count())
                .sum();
            if chars < 6 {
                continue;
            }
            let mut end = first + width;
            while end < items.len() && items[end].0 == items[first + (end - first) % width].0 {
                end += 1;
                let cycles = (end - first) / width;
                if cycles >= 3
                    && (chars * cycles >= 48 || items[end - 1].2 - items[first].1 >= 12_000_000)
                {
                    return true;
                }
            }
        }
    }
    false
}

// Catch a whole segment that repeats without punctuation as well.
fn periodic(text: &str, duration: i64) -> bool {
    let chars: Vec<_> = text.chars().collect();
    if chars.len() < 18 {
        return false;
    }
    let mut prefix = vec![0; chars.len()];
    for i in 1..chars.len() {
        let mut j = prefix[i - 1];
        while j > 0 && chars[i] != chars[j] {
            j = prefix[j - 1];
        }
        if chars[i] == chars[j] {
            j += 1;
        }
        prefix[i] = j;
    }
    let period = chars.len() - prefix[chars.len() - 1];
    period >= 6
        && chars.len() % period == 0
        && chars.len() / period >= 3
        && (chars.len() >= 48 || duration >= 12_000_000)
}

pub(super) fn reasons(
    segments: &[Segment],
    vocabulary: Option<&VocabularySnapshot>,
) -> Vec<Reason> {
    let whole: Vec<_> = segments
        .iter()
        .map(|s| (normalized(&s.text), s.start_us, s.end_us))
        .filter(|v| !v.0.is_empty())
        .collect();
    let sentences: Vec<_> = segments
        .iter()
        .flat_map(|s| {
            s.text
                .split(['。', '！', '？', '.', '!', '?', '\n'])
                .map(move |part| (normalized(part), s.start_us, s.end_us))
        })
        .filter(|v| !v.0.is_empty())
        .collect();
    let mut result = vec![];
    if repeated(&whole) || repeated(&sentences) || whole.iter().any(|v| periodic(&v.0, v.2 - v.1)) {
        result.push(Reason::Repetition);
    }
    if let Some(vocabulary) = vocabulary {
        let text = whole.iter().map(|v| v.0.as_str()).collect::<String>();
        if vocabulary
            .entries
            .iter()
            .filter_map(|e| e.entry.reading.as_deref())
            .map(normalized)
            .any(|r| r.chars().count() >= 12 && text.contains(&r))
        {
            result.push(Reason::ReadingEcho);
        }
        // Match adjacent copies of distinct dictionary items, allowing either
        // a bare term or the rendered term+reading. A single spoken term is OK.
        let patterns: Vec<_> = vocabulary
            .entries
            .iter()
            .enumerate()
            .flat_map(|(id, e)| {
                let term = normalized(&e.entry.term);
                let rendered = format!(
                    "{}{}",
                    term,
                    e.entry
                        .reading
                        .as_deref()
                        .map(normalized)
                        .unwrap_or_default()
                );
                [(id, term), (id, rendered)]
            })
            .filter(|(_, s)| !s.is_empty())
            .collect();
        let mut echo = false;
        'outer: for (id, pattern) in &patterns {
            for (at, _) in text.match_indices(pattern) {
                let tail = &text[at + pattern.len()..];
                if patterns.iter().any(|(other, next)| {
                    other != id
                        && pattern.chars().count() + next.chars().count() >= 24
                        && tail.starts_with(next)
                }) {
                    echo = true;
                    break 'outer;
                }
            }
        }
        if echo {
            result.push(Reason::VocabularyEcho);
        }
    }
    result
}

// Hints can also cause an initial utterance to collapse into a short fragment
// followed by a long timestamp jump, without any repeated words. Retry the
// whole window conservatively; genuine pauses remain intact in the unhinted
// result. This neither clips audio nor introduces a speech detector.
pub(super) fn hinted_reasons(
    segments: &[Segment],
    vocabulary: Option<&VocabularySnapshot>,
) -> Vec<Reason> {
    let mut result = reasons(segments, vocabulary);
    if result.is_empty() {
        let mut covered_end = None;
        for segment in segments {
            if covered_end.is_some_and(|end| segment.start_us - end >= 12_000_000) {
                result.push(Reason::TimestampGap);
                break;
            }
            covered_end = Some(covered_end.unwrap_or(0).max(segment.end_us));
        }
    }
    result
}

pub(super) struct Outcome {
    pub selected: PathBuf,
    pub rejected: Option<PathBuf>,
    pub reasons: Vec<Reason>,
}

pub(super) fn run(
    config: &AsrConfig,
    directory: &Path,
    duration: i64,
    suppress: bool,
    guarded: bool,
    vocabulary: Option<&VocabularySnapshot>,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Outcome>, mon_contracts::ProcessingError> {
    let Some(first) = whisper::run_unit_with_suppression(
        config,
        directory,
        duration,
        Duration::from_secs(1800),
        suppress,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    let problems = if guarded && !config.prompt.is_empty() {
        hinted_reasons(&first, vocabulary)
    } else if guarded {
        reasons(&first, None)
    } else {
        vec![]
    };
    if problems.is_empty() {
        return Ok(Some(Outcome {
            selected: directory.join("result.json"),
            rejected: None,
            reasons: problems,
        }));
    }
    if config.prompt.is_empty() {
        return Err(mon_contracts::ProcessingError::quality(
            "ヒントなしの文字起こしに反復を検出しました。異常な区間は採用していません",
            problems.iter().map(|r| format!("{r:?}")).collect(),
        ));
    }
    if cancelled() {
        return Ok(None);
    }
    let fallback = directory.join("without-hints");
    std::fs::create_dir(&fallback).map_err(|e| e.to_string())?;
    super::input::Input::load(directory, cancelled)?.write(&fallback)?;
    let mut plain = config.clone();
    plain.prompt.clear();
    let Some(second) = whisper::run_unit_with_suppression(
        &plain,
        &fallback,
        duration,
        Duration::from_secs(1800),
        suppress,
        cancelled,
    )?
    else {
        return Ok(None);
    };
    if !reasons(&second, None).is_empty() {
        return Err(mon_contracts::ProcessingError::quality(
            "ヒントなしで再認識しても反復が残りました。異常な区間は採用していません",
            reasons(&second, None)
                .iter()
                .map(|r| format!("{r:?}"))
                .collect(),
        ));
    }
    Ok(Some(Outcome {
        selected: fallback.join("result.json"),
        rejected: Some(directory.join("result.json")),
        reasons: problems,
    }))
}

#[cfg(test)]
mod tests;
