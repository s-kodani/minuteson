//! Legacy schema and citation-fragment preservation for replay. No output format validation.
pub(super) fn pattern() -> String {
    // llama.cpp's anchored pattern subset becomes a grammar. Keep it to
    // literals, character classes and repetition (no lookaround/backrefs).
    let text = r#"[^"\\\x00-\x20#。\[-][^"\\\x00-\x1F#。\[]*"#;
    let link = r"\[[0-9:.]+–[0-9:.]+\]\(audio:[0-9:.]+-[0-9:.]+\)";
    let paragraph = format!("({text}。({link})+)+");
    let item = format!("- {paragraph}");
    format!(
        r"^# 議事録\n\n## 要約\n{paragraph}(\n\n{paragraph})*(\n\n## 決定事項\n{item}(\n{item})*)?(\n\n## ToDo\n{item}(\n{item})*)?\n?$"
    )
}

pub(super) fn read_links(text: &str, strict: bool) -> Result<Vec<&str>, String> {
    let mut result = Vec::new();
    let mut remaining = text;
    while let Some(position) = remaining.find("](audio:") {
        let before = &remaining[..position];
        let bounds = before.rfind('[').zip(remaining[position..].find(')'));
        let Some((start, offset)) = bounds else {
            if strict {
                return Err("再生リンクの形式が不正です".into());
            }
            remaining = &remaining[position + 2..];
            continue;
        };
        let end = position + offset + 1;
        result.push(&remaining[start..end]);
        remaining = &remaining[end..];
    }
    Ok(result)
}

// UTF-8 compression may bisect a paragraph before its trailing citation.
// Carry that paragraph's original link alongside the exact source fragment.
pub(in crate::generation) fn fragment_links(
    text: &str,
    first: usize,
    end: usize,
) -> Result<Vec<String>, String> {
    let fragment = &text[first..end];
    let mut offset = 0;
    let mut result = Vec::new();
    for line in text.split_inclusive('\n') {
        let next = offset + line.len();
        if offset < end && next > first {
            for link in read_links(line, false)? {
                if !fragment.contains(link) && !result.iter().any(|s| s == link) {
                    result.push(link.to_owned());
                }
            }
        }
        offset = next;
    }
    Ok(result)
}
