//! Final V4 generation projection. Raw artifacts and human edits stay intact;
//! checkpoint replay/restore apply the same deterministic projection.
const MAX_CHARS: usize = 500;
fn heading(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start_matches(' ');
    if line.len() - text.len() > 3 {
        return None;
    }
    let text = text.trim_end();
    let level = text.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) || !text.get(level..)?.starts_with(char::is_whitespace) {
        return None;
    }
    Some((level, text[level..].trim().trim_end_matches('#').trim()))
}
fn summary_heading(line: &str) -> bool {
    heading(line).is_some_and(|(level, label)| {
        level >= 2 && matches!(label, "要約" | "要約（続）" | "要約(続)")
    })
}
fn body_chars(text: &str) -> usize {
    let count = |s: &str| s.chars().filter(|c| !matches!(c, '\n' | '\r')).count();
    count(text)
        - super::format::read_links(text, false)
            .unwrap_or_default()
            .iter()
            .map(|s| count(s))
            .sum::<usize>()
}
fn bounded(body: &str) -> String {
    if body_chars(body) <= MAX_CHARS {
        return body.trim().to_owned();
    }
    let mut search = 0;
    let spans = super::format::read_links(body, false)
        .unwrap_or_default()
        .iter()
        .map(|link| {
            let start = search + body[search..].find(link).unwrap();
            search = start + link.len();
            (start, search)
        })
        .collect::<Vec<_>>();
    let mut links = spans.into_iter().peekable();
    let mut offset = 0;
    let mut count = 0;
    let mut complete = None;
    let mut at_sentence_end = false;
    let mut fallback = 0;
    while offset < body.len() {
        if links.peek().is_some_and(|(start, _)| *start == offset) {
            offset = links.next().unwrap().1;
            if at_sentence_end {
                complete = Some(offset);
            }
            continue;
        }
        let c = body[offset..].chars().next().unwrap();
        if !matches!(c, '\n' | '\r') {
            if count == MAX_CHARS {
                break;
            }
            count += 1;
        }
        offset += c.len_utf8();
        if count < MAX_CHARS {
            fallback = offset;
        }
        if matches!(c, '。' | '！' | '？' | '.' | '!' | '?') {
            at_sentence_end = true;
        } else if !c.is_whitespace() {
            at_sentence_end = false;
        }
        if at_sentence_end {
            complete = Some(offset);
        }
    }
    if let Some(end) = complete {
        return body[..end].trim().to_owned();
    }
    // A single oversized sentence has no complete sentence boundary. Mark
    // its Unicode-safe excerpt and retain that paragraph's original links.
    let mut result = body[..fallback].trim_end().to_owned();
    result.push('…');
    let first_paragraph = body.split("\n\n").next().unwrap_or(body);
    for link in super::format::read_links(first_paragraph, false).unwrap_or_default() {
        if !result.contains(link) {
            result.push_str(link);
        }
    }
    result
}
pub(in crate::generation) fn limit(text: &str) -> String {
    let mut ranges = Vec::new();
    let mut active: Option<(usize, usize)> = None;
    let mut offset = 0;
    let mut summary_lines = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let marker = trimmed.as_bytes().first().copied().unwrap_or(0);
        let run = trimmed.bytes().take_while(|c| *c == marker).count();
        let fenced = fence.is_some();
        if line.len() - trimmed.len() <= 3 && matches!(marker, b'`' | b'~') && run >= 3 {
            match fence {
                Some((open, length))
                    if open == marker && run >= length && trimmed[run..].trim().is_empty() =>
                {
                    fence = None
                }
                None => fence = Some((marker, run)),
                _ => {}
            }
        }
        if fenced || fence.is_some() {
            offset += line.len();
            continue;
        }
        if summary_heading(line) {
            summary_lines.push(offset);
            if active.is_none() {
                active = Some((offset, heading(line).unwrap().0));
            }
        } else if let Some((start, level)) = active {
            if heading(line).is_some_and(|(next, _)| next <= level) {
                ranges.push((start, offset));
                active = None;
            }
        }
        offset += line.len();
    }
    if let Some((start, _)) = active {
        ranges.push((start, text.len()));
    }
    if ranges.is_empty() {
        return text.to_owned();
    }
    let body = ranges
        .iter()
        .map(|(start, end)| {
            let mut position = *start;
            text[*start..*end]
                .split_inclusive('\n')
                .filter_map(|line| {
                    let keep = summary_lines.binary_search(&position).is_err();
                    position += line.len();
                    keep.then_some(line)
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if summary_lines.len() == 1 && body_chars(body.trim()) <= MAX_CHARS {
        return text.to_owned();
    }
    let summary = bounded(body.trim());
    let mut result = String::new();
    let mut copied = 0;
    for (index, (start, end)) in ranges.into_iter().enumerate() {
        result.push_str(&text[copied..start]);
        if index == 0 {
            result.push_str("## 要約\n");
            result.push_str(&summary);
            result.push_str("\n\n");
        }
        copied = end;
    }
    result.push_str(&text[copied..]);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    const LINK: &str = "[00:01:47–00:01:54](audio:00:01:47-00:01:54)";
    #[test]
    fn only_summary_is_bounded_with_complete_sentences_and_exact_citations() {
        let sentence = format!("{}。{LINK}", "確".repeat(199));
        let tail = format!(
            "## 決定事項\n- {}。{LINK}\n\n## ToDo\n- {}。{LINK}\n",
            "決".repeat(600),
            "作".repeat(700)
        );
        let source = format!("# 議事録\n\n## 要約\n{}\n\n{tail}", sentence.repeat(4));
        let result = limit(&source);
        assert!(result.ends_with(&tail));
        let section = result
            .split("## 要約\n")
            .nth(1)
            .unwrap()
            .split("## 決定事項")
            .next()
            .unwrap()
            .trim();
        assert_eq!(body_chars(section), 400);
        assert_eq!(section, sentence.repeat(2));
        assert_eq!(limit(&result), result);
    }
    #[test]
    fn exact_boundary_ignores_links_and_counts_unicode_not_utf8_bytes() {
        let exact = format!("# 議事録\n\n## 要約\n{}。{LINK}\n", "あ".repeat(499));
        assert_eq!(limit(&exact), exact);
        let long = format!("# 議事録\n\n## 要約\n{}{LINK}\n", "😀".repeat(501));
        let result = limit(&long);
        let section = result.split("## 要約\n").nth(1).unwrap().trim();
        assert_eq!(body_chars(section), 500);
        assert!(section.ends_with(&format!("…{LINK}")));
        assert!(!section.contains('\u{fffd}'));
    }
    #[test]
    fn continued_and_repeated_summaries_share_one_budget_without_touching_other_sections() {
        let source=format!("# 議事録\n\n## 要約\n{}。{LINK}\n\n### 要約（続）\n{}。{LINK}\n\n## ToDo\n- 確認する。{LINK}\n\n## 要約\n{}。{LINK}\n", "甲".repeat(249),"乙".repeat(249),"丙".repeat(249));
        let result = limit(&source);
        assert_eq!(result.matches("## 要約\n").count(), 1);
        assert!(!result.contains("（続）"));
        assert!(!result.contains('丙'));
        assert!(result.contains(&format!("## ToDo\n- 確認する。{LINK}")));
    }
    #[test]
    fn no_summary_or_a_short_summary_is_left_byte_for_byte_unchanged() {
        for text in [
            "# 人間の本文\n長さ制限なし。",
            " \r\n# 議事録\r\n\r\n## 要約\r\n短くまとめる。\r\n\r\n## ToDo\r\n- 確認する。\r\n",
        ] {
            assert_eq!(limit(text), text);
        }
    }
    #[test]
    fn fenced_or_indented_summary_labels_in_other_sections_are_not_rewritten() {
        let tail="## ToDo\n- コードを確認する。\n\n```markdown\n## 要約\nこのコードはそのまま保持する。\n```\n\n    ## 要約\n    インデントしたコードも保持する。\n";
        let source = format!(
            "# 議事録\n\n## 要約\n{}\n\n{tail}",
            "確認する。".repeat(150)
        );
        let result = limit(&source);
        assert!(result.ends_with(tail));
    }
}
