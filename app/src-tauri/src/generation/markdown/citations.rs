use crate::storage::minutes_markdown::validate_text;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::generation) struct Warnings {
    missing_blocks: usize,
    invalid_references: bool,
}

impl Warnings {
    pub(in crate::generation) fn needs_review(self) -> bool {
        self.missing_blocks != 0 || self.invalid_references
    }
}

fn events(text: &str) -> impl Iterator<Item = (Event<'_>, Range<usize>)> {
    Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS).into_offset_iter()
}

// Reserve the {{A namespace. Consume a malformed identifier without eating
// neighboring prose, even when the model omitted the closing braces.
fn markers(text: &str) -> impl Iterator<Item = (Range<usize>, Option<usize>)> + '_ {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let start = offset + text[offset..].find("{{A")?;
        let value = &text[start + 3..];
        let len = value
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            .count();
        let tail = &value[len..];
        let braces = if tail.starts_with("}}") {
            2
        } else {
            usize::from(tail.starts_with('}'))
        };
        offset = start + 3 + len + braces;
        let id = if braces == 2 && !value.starts_with('0') {
            value[..len].parse::<usize>().ok().filter(|&id| id > 0)
        } else {
            None
        };
        Some((start..offset, id))
    })
}

pub(in crate::generation) fn inline_markers(text: &str) -> Vec<(Range<usize>, usize)> {
    let mut excluded = 0;
    let mut result = Vec::new();
    for (event, range) in events(text) {
        match event {
            Event::Start(
                Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::Link { .. }
                | Tag::Image { .. }
                | Tag::Heading { .. },
            ) => excluded += 1,
            Event::End(
                TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::Link
                | TagEnd::Image
                | TagEnd::Heading(_),
            ) => excluded -= 1,
            Event::Text(_) if excluded == 0 => {
                result.extend(markers(&text[range.clone()]).filter_map(|(marker, id)| {
                    id.map(|id| ((range.start + marker.start)..(range.start + marker.end), id))
                }));
            }
            _ => {}
        }
    }
    result
}

fn apply_edits(text: String, mut edits: Vec<(Range<usize>, String)>) -> String {
    if edits.is_empty() {
        return text;
    }
    edits.sort_unstable_by_key(|(range, _)| range.start);
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (range, replacement) in edits {
        if range.start >= cursor {
            output.push_str(&text[cursor..range.start]);
            output.push_str(&replacement);
            cursor = range.end;
        }
    }
    output.push_str(&text[cursor..]);
    output
}

pub(in crate::generation) fn normalize(text: String, allowed: &BTreeSet<usize>) -> (String, bool) {
    let mut edits = Vec::new();
    let mut excluded = 0;
    let mut literal = 0;
    let mut audio_link: Option<(Range<usize>, Option<Range<usize>>)> = None;
    for (event, range) in events(&text) {
        if let Some((link, label)) = &mut audio_link {
            if event == Event::End(TagEnd::Link) {
                let label = label.take().map_or("", |label| &text[label]);
                let label = apply_edits(
                    label.to_owned(),
                    markers(label)
                        .map(|(marker, _)| (marker, String::new()))
                        .collect(),
                );
                edits.push((link.clone(), label));
                audio_link = None;
            } else {
                let label = label.get_or_insert(range.clone());
                label.start = label.start.min(range.start);
                label.end = label.end.max(range.end);
            }
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) => literal += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock) => literal -= 1,
            Event::Start(Tag::Link { dest_url, .. })
                if dest_url
                    .get(..6)
                    .is_some_and(|s| s.eq_ignore_ascii_case("audio:")) =>
            {
                audio_link = Some((range, None));
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::Heading { .. }) => {
                excluded += 1
            }
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::Heading(_)) => excluded -= 1,
            Event::Code(code)
                if literal == 0
                    && markers(&code)
                        .next()
                        .is_some_and(|(marker, _)| marker == (0..code.len())) =>
            {
                edits.push((range, String::new()));
            }
            Event::Text(_) if literal == 0 => {
                for (marker, id) in markers(&text[range.clone()]) {
                    if excluded > 0 || !id.is_some_and(|id| allowed.contains(&id)) {
                        let start = range.start + marker.start;
                        let end = range.start + marker.end;
                        // Removing an invalid marker must not join a label
                        // to an untrusted Markdown link destination.
                        let separator =
                            if text[..start].ends_with(']') && text[end..].starts_with('(') {
                                " "
                            } else {
                                ""
                            };
                        edits.push((start..end, separator.into()));
                    }
                }
            }
            _ => {}
        }
    }
    let invalid = !edits.is_empty();
    (apply_edits(text, edits), invalid)
}

#[derive(Default)]
struct Block {
    has_text: bool,
    has_citation: bool,
}

fn missing_blocks(text: &str) -> usize {
    let mut blocks = Vec::new();
    let mut excluded = 0;
    let mut paragraphs = Vec::new();
    let mut missing = 0;
    for (event, range) in events(text) {
        match event {
            Event::Start(
                Tag::Heading { .. } | Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::TableHead,
            ) => excluded += 1,
            Event::End(
                TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::TableHead,
            ) => excluded -= 1,
            _ if excluded != 0 => {}
            Event::Start(Tag::Item | Tag::TableRow) => blocks.push(Block::default()),
            Event::Start(Tag::Paragraph) => {
                paragraphs.push(blocks.is_empty());
                if blocks.is_empty() {
                    blocks.push(Block::default());
                }
            }
            Event::End(TagEnd::Item | TagEnd::TableRow) => {
                let block = blocks.pop().expect("balanced Markdown blocks");
                missing += usize::from(block.has_text && !block.has_citation);
            }
            Event::End(TagEnd::Paragraph) => {
                if paragraphs.pop() == Some(true) {
                    let block = blocks.pop().expect("balanced Markdown paragraphs");
                    missing += usize::from(block.has_text && !block.has_citation);
                }
            }
            Event::Text(_) => {
                if let Some(block) = blocks.last_mut() {
                    let text = &text[range];
                    let mut cursor = 0;
                    for (range, id) in markers(text) {
                        block.has_text |= !text[cursor..range.start].trim().is_empty();
                        block.has_citation |= id.is_some();
                        cursor = range.end;
                    }
                    block.has_text |= !text[cursor..].trim().is_empty();
                }
            }
            Event::Code(code) => {
                if let Some(block) = blocks.last_mut() {
                    block.has_text |= !code.trim().is_empty();
                }
            }
            _ => {}
        }
    }
    missing
}

pub(in crate::generation) fn resolve(
    text: String,
    invalid_references: bool,
    mut link: impl FnMut(usize) -> Result<String, String>,
) -> Result<(String, Warnings), String> {
    let warnings = Warnings {
        missing_blocks: missing_blocks(&text),
        invalid_references,
    };
    let edits = inline_markers(&text)
        .into_iter()
        .map(|(range, id)| Ok((range, link(id)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let markdown = apply_edits(text, edits);
    validate_text(&markdown)?;
    if markdown.trim().is_empty() {
        return Err("議事録の生成結果が空です".into());
    }
    Ok((markdown, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_preserve_format_and_warn_per_body_block() {
        let text = "# 会議\n\n確認した。{{A1}}{{A2}}\n\n- 実施する。{{A1}}\n- [00:01:14–00:01:25]\n\n| 項目 | 結果 |\n| --- | --- |\n| 点検 | 済み{{A2}} |\n| 工事 | 未定 |\n\n```text\n{{A3}}\n```\n";
        let (text, invalid) = normalize(text.into(), &BTreeSet::from([1, 2]));
        assert!(!invalid);
        let (markdown, warnings) = resolve(text, invalid, |id| {
            Ok(format!("[引用{id}](audio:00:01:14-00:01:25)"))
        })
        .unwrap();
        assert_eq!(
            warnings,
            Warnings {
                missing_blocks: 2,
                invalid_references: false
            }
        );
        assert!(markdown.contains(
            "確認した。[引用1](audio:00:01:14-00:01:25)[引用2](audio:00:01:14-00:01:25)"
        ));
        assert!(markdown.contains("| 点検 | 済み[引用2](audio:00:01:14-00:01:25) |"));
        assert!(markdown.contains("```text\n{{A3}}\n```"));
    }

    #[test]
    fn citations_remove_untrusted_links_and_markers_without_dropping_prose() {
        let input = "前。{{A0}}{{A02}}{{A3}}{{A9 後。{{A1}} [**点検**](audio:00:00:01-00:00:02) [確認][ref]`{{A2}}`\n\n[ref]: audio:00:01:14-00:01:25\n";
        let (text, invalid) = normalize(input.into(), &BTreeSet::from([1, 2]));
        assert!(invalid);
        assert!(
            text.starts_with("前。 後。{{A1}} **点検** 確認\n"),
            "{text:?}"
        );
        let (markdown, warnings) = resolve(text, invalid, |_| {
            Ok("[引用](audio:00:01:14-00:01:25)".into())
        })
        .unwrap();
        assert!(!markdown.contains("{{A"));
        assert!(warnings.needs_review());
        assert_eq!(warnings.missing_blocks, 0);
    }
}
