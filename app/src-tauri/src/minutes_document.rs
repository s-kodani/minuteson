use crate::storage::MinutesInput;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(crate) const TEMPLATE_VERSION: &str = "standard-v1";
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ItemKind {
    Summary,
    Topic,
    Decision,
    OpenIssue,
    Todo,
}
impl ItemKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Topic => "topic",
            Self::Decision => "decision",
            Self::OpenIssue => "open_issue",
            Self::Todo => "todo",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceType {
    Engine,
    Manual,
}
impl SourceType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Manual => "manual",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerificationState {
    Unverified,
    UserVerified,
}
impl VerificationState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::UserVerified => "user_verified",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesItem {
    pub id: String,
    pub kind: ItemKind,
    pub body: String,
    pub assignee_name: Option<String>,
    pub assignee_speaker_id: Option<String>,
    pub due_date: Option<String>,
    pub due_text: Option<String>,
    pub source_type: SourceType,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Evidence {
    pub id: String,
    pub minutes_item_id: String,
    pub analysis_revision_id: String,
    pub utterance_id: String,
    pub quote_text: String,
    pub start_us: i64,
    pub end_us: i64,
    pub source_type: SourceType,
    pub verification_state: VerificationState,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Inline {
    Text {
        text: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
    },
    Evidence {
        #[serde(rename = "minutesItemId")]
        minutes_item_id: String,
        #[serde(rename = "evidenceLinkId")]
        evidence_link_id: String,
    },
}
impl Inline {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            bold: false,
            italic: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TextBlock {
    pub item_id: Option<String>,
    pub content: Vec<Inline>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TableRow {
    pub item_id: Option<String>,
    pub cells: Vec<Vec<Inline>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Block {
    Heading {
        level: u8,
        #[serde(default, rename = "itemId", skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        content: Vec<Inline>,
    },
    Paragraph {
        #[serde(rename = "itemId")]
        item_id: Option<String>,
        content: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<TextBlock>,
    },
    Table {
        headers: Vec<Vec<Inline>>,
        rows: Vec<TableRow>,
    },
}
// One canonical, versioned document with annotations. SQL items and evidence
// are validated projections, not independently editable copies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesContent {
    pub schema_version: u32,
    pub blocks: Vec<Block>,
    pub items: Vec<MinutesItem>,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct GeneratedItem {
    pub kind: ItemKind,
    pub body: String,
    pub assignee_name: Option<String>,
    pub assignee_speaker_id: Option<String>,
    pub due_date: Option<String>,
    pub due_text: Option<String>,
    pub evidence_utterance_ids: Vec<String>,
}

#[allow(dead_code)]
pub(crate) fn from_generated(
    input: &MinutesInput,
    raw: Vec<GeneratedItem>,
) -> Result<MinutesContent, String> {
    if raw.len() > 10_000 {
        return Err("生成項目が多すぎます".into());
    }
    let utterances: HashMap<_, _> = input
        .analysis
        .utterances
        .iter()
        .map(|u| (u.id.as_str(), u))
        .collect();
    let mut items = vec![];
    let mut evidence = vec![];
    let mut retained_bytes = 0_usize;
    for raw in raw {
        if raw.body.trim().is_empty()
            || !valid_text(&raw.body, 20_000)
            || [&raw.assignee_name, &raw.due_text].iter().any(|v| {
                v.as_ref()
                    .is_some_and(|s| s.trim().is_empty() || !valid_text(s, 200))
            })
            || raw.due_date.as_ref().is_some_and(|s| !valid_date(s))
        {
            return Err("生成項目の本文・担当者・期限が不正です".into());
        }
        retained_bytes = retained_bytes
            .saturating_add(raw.body.len())
            .saturating_add(512);
        if retained_bytes > MAX_CONTENT_BYTES {
            return Err("生成文書が大きすぎます".into());
        }
        if raw.evidence_utterance_ids.is_empty() || raw.evidence_utterance_ids.len() > 100 {
            return Err("生成項目には固定入力の根拠が必要です".into());
        }
        let id = Uuid::new_v4().to_string();
        let mut seen = HashSet::new();
        for utterance_id in raw.evidence_utterance_ids {
            if !seen.insert(utterance_id.clone()) {
                return Err("生成根拠が重複しています".into());
            }
            let u = utterances
                .get(utterance_id.as_str())
                .ok_or("生成根拠の発言IDが固定入力にありません")?;
            if !valid_text(&u.effective_text, 20_000) || u.effective_text.trim().is_empty() {
                return Err("空または未対応の発言は根拠にできません".into());
            }
            retained_bytes = retained_bytes
                .saturating_add(u.effective_text.len())
                .saturating_add(512);
            if retained_bytes > MAX_CONTENT_BYTES || evidence.len() >= 50_000 {
                return Err("生成根拠が大きすぎます".into());
            }
            evidence.push(Evidence {
                id: Uuid::new_v4().to_string(),
                minutes_item_id: id.clone(),
                analysis_revision_id: input.analysis.id.clone(),
                utterance_id,
                quote_text: u.effective_text.clone(),
                start_us: u.start_us,
                end_us: u.end_us,
                source_type: SourceType::Engine,
                verification_state: VerificationState::Unverified,
            });
        }
        items.push(MinutesItem {
            id,
            kind: raw.kind,
            body: raw.body,
            assignee_name: raw.assignee_name,
            assignee_speaker_id: raw.assignee_speaker_id,
            due_date: raw.due_date,
            due_text: raw.due_text,
            source_type: SourceType::Engine,
        });
    }
    let mut blocks = vec![heading(1, &input.meeting_title)];
    let date = input
        .meeting_started_at_utc_ms
        .and_then(utc_label)
        .unwrap_or_else(|| "未確認".into());
    let participants = input
        .analysis
        .speakers
        .iter()
        .filter_map(|s| s.display_name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    blocks.push(paragraph(format!("会議日時: {date}")));
    blocks.push(paragraph(format!(
        "確認済み参加者: {}",
        if participants.is_empty() {
            "未確認".into()
        } else {
            participants.join("、")
        }
    )));
    for (kind, label) in [
        (ItemKind::Summary, "概要"),
        (ItemKind::Topic, "議題・論点"),
        (ItemKind::Decision, "決定事項"),
        (ItemKind::OpenIssue, "課題・未決事項"),
        (ItemKind::Todo, "ToDo"),
    ] {
        blocks.push(heading(2, label));
        let selected = items.iter().filter(|i| i.kind == kind).collect::<Vec<_>>();
        if kind == ItemKind::Todo {
            let rows = selected
                .iter()
                .map(|item| TableRow {
                    item_id: Some(item.id.clone()),
                    cells: vec![
                        vec![Inline::text(&item.body)],
                        vec![Inline::text(assignee_label(item))],
                        vec![Inline::text(due_label(item))],
                        evidence_chips(item, &evidence),
                    ],
                })
                .collect();
            blocks.push(Block::Table {
                headers: ["内容", "担当者", "期限", "根拠"]
                    .iter()
                    .map(|s| vec![Inline::text(*s)])
                    .collect(),
                rows,
            });
        } else if !selected.is_empty() {
            blocks.push(Block::List {
                ordered: false,
                items: selected
                    .iter()
                    .map(|item| {
                        let mut content = vec![Inline::text(&item.body)];
                        content.extend(evidence_chips(item, &evidence));
                        TextBlock {
                            item_id: Some(item.id.clone()),
                            content,
                        }
                    })
                    .collect(),
            });
        }
        if selected.is_empty() {
            blocks.push(paragraph("抽出項目なし（内容を確認してください）"));
        }
    }
    blocks.push(paragraph(
        "AI生成結果です。決定事項・担当者・期限を根拠の音声と照合してください。",
    ));
    let result = MinutesContent {
        schema_version: 1,
        blocks,
        items,
        evidence,
    };
    result.validate(input)?;
    Ok(result)
}
fn heading(level: u8, text: &str) -> Block {
    Block::Heading {
        level,
        item_id: None,
        content: vec![Inline::text(text)],
    }
}
fn paragraph(text: impl Into<String>) -> Block {
    Block::Paragraph {
        item_id: None,
        content: vec![Inline::text(text)],
    }
}
fn evidence_chips(item: &MinutesItem, evidence: &[Evidence]) -> Vec<Inline> {
    evidence
        .iter()
        .filter(|e| e.minutes_item_id == item.id)
        .map(|e| Inline::Evidence {
            minutes_item_id: item.id.clone(),
            evidence_link_id: e.id.clone(),
        })
        .collect()
}
pub(crate) fn assignee_label(item: &MinutesItem) -> &str {
    item.assignee_name.as_deref().unwrap_or("未確認")
}
pub(crate) fn due_label(item: &MinutesItem) -> String {
    match (&item.due_date, &item.due_text) {
        (Some(date), Some(text)) if date != text => format!("{date}（{text}）"),
        (Some(date), _) => date.clone(),
        (_, Some(text)) => text.clone(),
        _ => "未確認".into(),
    }
}

impl MinutesContent {
    pub(crate) fn validate(&self, input: &MinutesInput) -> Result<(), String> {
        if self.schema_version != 1
            || self.blocks.len() > 10_000
            || self.items.len() > 10_000
            || self.evidence.len() > 50_000
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_CONTENT_BYTES
        {
            return Err("議事録の形式・件数・サイズが不正です".into());
        }
        let mut items = HashMap::new();
        for item in &self.items {
            if !valid_id(&item.id)
                || items.insert(item.id.as_str(), item).is_some()
                || item.body.trim().is_empty()
                || !valid_text(&item.body, 20_000)
            {
                return Err("議事録項目のID・本文が不正です".into());
            }
            for value in [&item.assignee_name, &item.due_text] {
                if value
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty() || !valid_text(s, 200))
                {
                    return Err("担当者・期限の原表現が不正です".into());
                }
            }
            if item.kind != ItemKind::Todo
                && (item.assignee_name.is_some()
                    || item.assignee_speaker_id.is_some()
                    || item.due_date.is_some()
                    || item.due_text.is_some())
            {
                return Err("ToDo以外に担当者・期限は指定できません".into());
            }
            if item.due_date.as_ref().is_some_and(|s| !valid_date(s)) {
                return Err("期限の日付が不正です".into());
            }
            if let Some(id) = &item.assignee_speaker_id {
                let s = input
                    .analysis
                    .speakers
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or("担当者の話者IDが固定入力にありません")?;
                if item.assignee_name.is_none()
                    || (item.source_type == SourceType::Engine
                        && s.display_name != item.assignee_name)
                {
                    return Err("担当者名と確認済み話者名が一致しません".into());
                }
            }
        }
        let utterances: HashMap<_, _> = input
            .analysis
            .utterances
            .iter()
            .map(|u| (u.id.as_str(), u))
            .collect();
        let mut links = HashMap::new();
        for e in &self.evidence {
            let item = items
                .get(e.minutes_item_id.as_str())
                .ok_or("根拠の項目IDがありません")?;
            let u = utterances
                .get(e.utterance_id.as_str())
                .ok_or("根拠の発言IDが固定入力にありません")?;
            if !valid_id(&e.id)
                || links.insert(e.id.as_str(), e).is_some()
                || e.analysis_revision_id != input.analysis.id
                || e.start_us < u.start_us
                || e.end_us > u.end_us
                || e.end_us <= e.start_us
                || e.quote_text.trim().is_empty()
                || !valid_text(&e.quote_text, 20_000)
                || !u.effective_text.contains(&e.quote_text)
                || e.source_type != item.source_type
            {
                return Err("根拠のID・版・時刻・引用が固定入力と一致しません".into());
            }
        }
        for item in &self.items {
            let refs = self
                .evidence
                .iter()
                .filter(|e| e.minutes_item_id == item.id)
                .collect::<Vec<_>>();
            if item.source_type == SourceType::Engine {
                if refs.is_empty() {
                    return Err("AI生成項目には根拠が必要です".into());
                }
                // This proves literal grounding only, not the semantic correctness
                // of assigning a task. The result remains explicitly unverified.
                for value in [&item.assignee_name, &item.due_date, &item.due_text] {
                    if value
                        .as_ref()
                        .is_some_and(|s| !refs.iter().any(|e| e.quote_text.contains(s)))
                    {
                        return Err(
                            "担当者・期限の値が引用にありません。未確認として扱ってください".into(),
                        );
                    }
                }
            }
        }
        let mut state = DocumentValidation {
            items: &items,
            links: &links,
            seen_items: HashSet::new(),
            seen_links: HashSet::new(),
            cells: 0,
        };
        for block in &self.blocks {
            match block {
                Block::Heading {
                    level,
                    item_id,
                    content,
                } => {
                    if !(1..=6).contains(level) {
                        return Err("見出しのレベルが不正です".into());
                    }
                    state.text_block(item_id.as_deref(), content)?;
                }
                Block::Paragraph { item_id, content } => {
                    state.text_block(item_id.as_deref(), content)?
                }
                Block::List { items, .. } => {
                    if items.len() > 10_000 {
                        return Err("箇条書きが多すぎます".into());
                    }
                    for item in items {
                        state.text_block(item.item_id.as_deref(), &item.content)?;
                    }
                }
                Block::Table { headers, rows } => {
                    if headers.is_empty() || headers.len() > 32 || rows.len() > 10_000 {
                        return Err("表の列・行数が不正です".into());
                    }
                    for cell in headers {
                        state.inline(cell, None)?;
                    }
                    for row in rows {
                        if row.cells.len() != headers.len() {
                            return Err("表の列数が一致しません".into());
                        }
                        for cell in &row.cells {
                            state.inline(cell, row.item_id.as_deref())?;
                        }
                        if let Some(id) = row.item_id.as_deref() {
                            let item = state.bind(id, &plain(&row.cells[0]))?;
                            if item.kind != ItemKind::Todo
                                || row.cells.len() != 4
                                || plain(&row.cells[1]) != assignee_label(item)
                                || plain(&row.cells[2]) != due_label(item)
                                || !plain(&row.cells[3]).trim().is_empty()
                            {
                                return Err("ToDo表と構造化項目が一致しません".into());
                            }
                        }
                    }
                }
            }
        }
        if state.seen_items.len() != items.len() || state.seen_links.len() != links.len() {
            return Err("文書に表示されない項目・根拠があります".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn markdown(&self, input: &MinutesInput) -> Result<String, String> {
        self.validate(input)?;
        let links: HashMap<_, _> = self.evidence.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut rendered = Markdown {
            links,
            used: vec![],
        };
        let mut output = String::new();
        for block in &self.blocks {
            match block {
                Block::Heading { level, content, .. } => output.push_str(&format!(
                    "{} {}\n\n",
                    "#".repeat(usize::from(*level)),
                    rendered.inline(content, false)
                )),
                Block::Paragraph { content, .. } => {
                    output.push_str(&format!("{}\n\n", rendered.inline(content, false)))
                }
                Block::List { ordered, items } => {
                    for (i, item) in items.iter().enumerate() {
                        let marker = if *ordered {
                            format!("{}. ", i + 1)
                        } else {
                            "- ".into()
                        };
                        let text = rendered.inline(&item.content, false);
                        output.push_str(&marker);
                        output.push_str(
                            &text.replace('\n', &format!("\n{}", " ".repeat(marker.len()))),
                        );
                        output.push('\n');
                    }
                    output.push('\n');
                }
                Block::Table { headers, rows } => {
                    output.push_str(&format!(
                        "| {} |\n",
                        headers
                            .iter()
                            .map(|c| rendered.inline(c, true))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ));
                    output.push_str(&format!("| {} |\n", vec!["---"; headers.len()].join(" | ")));
                    for row in rows {
                        output.push_str(&format!(
                            "| {} |\n",
                            row.cells
                                .iter()
                                .map(|c| rendered.inline(c, true))
                                .collect::<Vec<_>>()
                                .join(" | ")
                        ));
                    }
                    output.push('\n');
                }
            }
        }
        for (index, id) in rendered.used.iter().enumerate() {
            let e = rendered.links[id.as_str()];
            output.push_str(&format!(
                "[^根拠{}]: {}–{} · {}  \n    解析版: {} · 発言ID: {} · {}\n\n",
                index + 1,
                time_label(e.start_us),
                time_label(e.end_us),
                escape(&e.quote_text).replace('\n', " "),
                escape(&e.analysis_revision_id),
                escape(&e.utterance_id),
                if e.verification_state == VerificationState::UserVerified {
                    "利用者確認済み"
                } else {
                    "内容未確認"
                }
            ));
        }
        Ok(output)
    }
}

struct DocumentValidation<'a> {
    items: &'a HashMap<&'a str, &'a MinutesItem>,
    links: &'a HashMap<&'a str, &'a Evidence>,
    seen_items: HashSet<&'a str>,
    seen_links: HashSet<&'a str>,
    cells: usize,
}
impl<'a> DocumentValidation<'a> {
    fn bind(&mut self, id: &str, body: &str) -> Result<&'a MinutesItem, String> {
        let item = *self.items.get(id).ok_or("文書の項目IDがありません")?;
        if !self.seen_items.insert(item.id.as_str()) || item.body != body {
            return Err("文書の本文と項目が一致しないか、項目が重複しています".into());
        }
        Ok(item)
    }
    fn text_block(&mut self, owner: Option<&str>, content: &[Inline]) -> Result<(), String> {
        self.inline(content, owner)?;
        if let Some(id) = owner {
            if self.bind(id, &plain(content))?.kind == ItemKind::Todo {
                return Err("ToDoは担当者・期限を含む表にしてください".into());
            }
        }
        Ok(())
    }
    fn inline(&mut self, content: &[Inline], owner: Option<&str>) -> Result<(), String> {
        self.cells += 1;
        if self.cells > 200_000 || content.len() > 512 {
            return Err("文書要素が多すぎます".into());
        }
        for inline in content {
            match inline {
                Inline::Text { text, .. } => {
                    if !valid_text(text, 20_000) {
                        return Err("文書本文が不正です".into());
                    }
                }
                Inline::Evidence {
                    minutes_item_id,
                    evidence_link_id,
                } => {
                    let e = *self
                        .links
                        .get(evidence_link_id.as_str())
                        .ok_or("文書の根拠IDがありません")?;
                    if owner != Some(minutes_item_id.as_str())
                        || e.minutes_item_id != *minutes_item_id
                    {
                        return Err("根拠チップと文書項目が一致しません".into());
                    }
                    self.seen_links.insert(e.id.as_str());
                }
            }
        }
        Ok(())
    }
}
fn plain(content: &[Inline]) -> String {
    content
        .iter()
        .filter_map(|i| match i {
            Inline::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn valid_text(text: &str, max: usize) -> bool {
    text.chars().count() <= max
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
pub(crate) fn valid_date(date: &str) -> bool {
    if date.len() != 10
        || date.as_bytes()[4] != b'-'
        || date.as_bytes()[7] != b'-'
        || date
            .bytes()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year = date[0..4].parse::<u32>().unwrap_or(0);
    let month = date[5..7].parse::<u32>().unwrap_or(0);
    let day = date[8..10].parse::<u32>().unwrap_or(0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    year > 0 && day > 0 && day <= max
}
#[cfg(test)]
fn escape(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            result.push('\\');
        }
        result.push(c);
    }
    result
}
#[cfg(test)]
fn time_label(us: i64) -> String {
    let seconds = us / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn utc_label(ms: i64) -> Option<String> {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86400);
    let daytime = seconds.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(1..=9999).contains(&year) {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        daytime / 3600,
        daytime / 60 % 60,
        daytime % 60
    ))
}
#[cfg(test)]
struct Markdown<'a> {
    links: HashMap<&'a str, &'a Evidence>,
    used: Vec<String>,
}
#[cfg(test)]
impl Markdown<'_> {
    fn inline(&mut self, content: &[Inline], table: bool) -> String {
        let mut output = String::new();
        for i in content {
            match i {
                Inline::Text { text, bold, italic } => {
                    let mut formatted = escape(text).replace('\t', "    ");
                    if *bold || *italic {
                        let middle = formatted.trim();
                        let leading = formatted.len() - formatted.trim_start().len();
                        let trailing = formatted.trim_end().len();
                        if !middle.is_empty() {
                            let mark = if *bold && *italic {
                                "***"
                            } else if *bold {
                                "**"
                            } else {
                                "*"
                            };
                            formatted = format!(
                                "{}{mark}{middle}{mark}{}",
                                &formatted[..leading],
                                &formatted[trailing..]
                            );
                        }
                    }
                    output.push_str(&formatted.replace('\n', if table { "<br>" } else { "  \n" }));
                }
                Inline::Evidence {
                    evidence_link_id, ..
                } => {
                    let evidence = self.links[evidence_link_id.as_str()];
                    let index = match self.used.iter().position(|id| id == evidence_link_id) {
                        Some(i) => i,
                        None => {
                            self.used.push(evidence_link_id.clone());
                            self.used.len() - 1
                        }
                    };
                    output.push_str(&format!(
                        " {}[^根拠{}]",
                        time_label(evidence.start_us),
                        index + 1
                    ));
                }
            }
        }
        output
    }
}

#[cfg(test)]
#[path = "minutes_document_tests.rs"]
pub(crate) mod tests;
