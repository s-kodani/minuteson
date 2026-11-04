//! Durable, local person registry; source meetings do not own registered samples.
use super::*;
use crate::voice::{self, Plan, Window};
use rusqlite::{Transaction, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) mod audio;
mod enrollment;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersonMutation {
    pub audio_cleanup_pending: bool,
    pub audio_cleanup_error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioStorageState {
    pub cleanup_pending: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleView {
    pub id: String,
    pub model_hash: String,
    pub extractor_version: String,
    pub source_meeting_id: Option<String>,
    pub source_available: bool,
    pub audio_available: bool,
    pub audio_storage_state: String,
    pub audio_storage_error: Option<String>,
    pub start_us: i64,
    pub end_us: i64,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Person {
    pub id: String,
    pub name: String,
    pub version: i64,
    pub samples: Vec<SampleView>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Match {
    pub id: String,
    pub speaker_id: String,
    pub person_id: Option<String>,
    pub name_snapshot: String,
    pub state: String,
    pub similarity: Option<f64>,
    pub model_hash: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Context {
    pub supported: bool,
    pub analysis_id: String,
    pub working_hash: String,
    pub book_hash: String,
    pub people: Vec<Person>,
    pub matches: Vec<Match>,
    pub windows: Vec<Window>,
    pub registration_candidates: Vec<enrollment::Candidate>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Start {
    pub meeting_id: String,
    pub operation_id: String,
    pub analysis_id: String,
    pub working_hash: String,
    pub book_hash: String,
    pub speaker_id: Option<String>,
    pub person_id: Option<String>,
    pub person_name: Option<String>,
    // Omission must preserve the canonical JSON of existing fixed requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_windows: Option<Vec<Window>>,
}

fn people(db: &Connection) -> Result<Vec<Person>, String> {
    let mut query = db
        .prepare("SELECT id,name,version FROM voice_people ORDER BY name,id LIMIT 1001")
        .map_err(|e| e.to_string())?;
    let mut people = query
        .query_map([], |r| {
            Ok(Person {
                id: r.get(0)?,
                name: r.get(1)?,
                version: r.get(2)?,
                samples: vec![],
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if people.len() > 1000 {
        return Err("登録した人物が上限を超えています".into());
    }
    for person in &mut people {
        let mut q=db.prepare("SELECT s.id,s.model_hash,s.extractor_version,s.source_meeting_id,EXISTS(SELECT 1 FROM meeting_audio_commits a JOIN audio_chunks c ON c.track_id=a.track_id JOIN file_assets f ON f.id=c.file_asset_id WHERE a.meeting_id=s.source_meeting_id AND a.phase='cleaned' AND f.state='ready'),s.start_us,s.end_us,s.created_at,coalesce(a.state,'pending'),a.error_message FROM voice_samples s LEFT JOIN voice_sample_audio a ON a.id=s.id WHERE person_id=?1 ORDER BY s.created_at,s.id").map_err(|e|e.to_string())?;
        person.samples = q
            .query_map([&person.id], |r| {
                let state: String = r.get(8)?;
                let source_available: bool = r.get(4)?;
                let audio_available =
                    state == "ready" || (state != "unavailable" && source_available);
                Ok(SampleView {
                    id: r.get(0)?,
                    model_hash: r.get(1)?,
                    extractor_version: r.get(2)?,
                    source_meeting_id: r.get(3)?,
                    source_available,
                    start_us: r.get(5)?,
                    end_us: r.get(6)?,
                    created_at: r.get(7)?,
                    audio_available,
                    audio_storage_state: state,
                    audio_storage_error: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
    }
    Ok(people)
}
fn matches(db: &Connection, analysis: &str) -> Result<Vec<Match>, String> {
    let mut q=db.prepare("SELECT id,speaker_id,person_id,name_snapshot,state,similarity,model_hash FROM voice_matches WHERE analysis_revision_id=?1 ORDER BY speaker_id").map_err(|e|e.to_string())?;
    let result = q
        .query_map([analysis], |r| {
            Ok(Match {
                id: r.get(0)?,
                speaker_id: r.get(1)?,
                person_id: r.get(2)?,
                name_snapshot: r.get(3)?,
                state: r.get(4)?,
                similarity: r.get(5)?,
                model_hash: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(result)
}
pub(crate) fn validate_registry(db: &Connection) -> Result<(), String> {
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM voice_samples", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count > 8192 {
        return Err("登録した声が上限を超えています".into());
    }
    let mixed_dimensions:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM voice_samples GROUP BY model_hash,extractor_version HAVING MIN(dimension)!=MAX(dimension))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if mixed_dimensions {
        return Err("同じモデルの声の特徴量の次元が一致しません".into());
    }
    let mut policy_query = db
        .prepare("SELECT model_hash,extractor_version,evaluation_hash FROM voice_policies")
        .map_err(|e| e.to_string())?;
    let mut policies = policy_query.query([]).map_err(|e| e.to_string())?;
    while let Some(r) = policies.next().map_err(|e| e.to_string())? {
        let model: String = r.get(0).map_err(|e| e.to_string())?;
        let extractor: String = r.get(1).map_err(|e| e.to_string())?;
        let evaluation: String = r.get(2).map_err(|e| e.to_string())?;
        if !jobs::valid_hash(&model)
            || !jobs::valid_hash(&evaluation)
            || extractor != voice::EXTRACTOR
        {
            return Err("声の自動識別の評価条件が不正です".into());
        }
    }
    for person in people(db)? {
        valid_name(&person.name)?;
        if person.samples.len() > 64 {
            return Err("一人の登録音声が上限を超えています".into());
        }
    }
    let mut q = db
        .prepare("SELECT dimension,vector,model_hash,extractor_version FROM voice_samples")
        .map_err(|e| e.to_string())?;
    let mut rows = q.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let dim: u32 = row.get(0).map_err(|e| e.to_string())?;
        let bytes: Vec<u8> = row.get(1).map_err(|e| e.to_string())?;
        voice::decode(&bytes, dim as usize)?;
        let hash: String = row.get(2).map_err(|e| e.to_string())?;
        let version: String = row.get(3).map_err(|e| e.to_string())?;
        if !jobs::valid_hash(&hash) || version != voice::EXTRACTOR {
            return Err("登録した声のモデル・抽出方式が不正です".into());
        }
    }
    Ok(())
}
pub(super) fn book_hash(db: &Connection) -> Result<String, String> {
    validate_registry(db)?;
    let mut bytes = serde_json::to_vec(&people(db)?).map_err(|e| e.to_string())?;
    for sql in ["SELECT id,hex(vector) FROM voice_samples ORDER BY id","SELECT id,model_hash,extractor_version,threshold,margin,evaluation_hash FROM voice_policies ORDER BY id"] {
        let mut q=db.prepare(sql).map_err(|e|e.to_string())?;let n=q.column_count();let mut rows=q.query([]).map_err(|e|e.to_string())?;
        while let Some(row)=rows.next().map_err(|e|e.to_string())? {
            let values=(0..n).map(|i|row.get::<_,rusqlite::types::Value>(i)).collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            bytes.extend_from_slice(format!("{values:?}").as_bytes());
        }
    }
    Ok(jobs::hash_bytes(b"mon-voice-book-v1", &bytes))
}
pub(super) fn working(
    tx: &Transaction<'_>,
    meeting: &str,
) -> Result<(AnalysisView, Option<TranscriptDraft>, String), String> {
    let (mut view, draft, _, _, _) = minutes_input::current_working(tx, meeting)?;
    let hash = jobs::hash_bytes(
        b"mon-voice-working-v1",
        &serde_json::to_vec(&(&view, &draft)).map_err(|e| e.to_string())?,
    );
    if let Some(d) = &draft {
        minutes_input::apply_edits(&mut view, &d.payload);
    }
    Ok((view, draft, hash))
}
fn windows(tx: &Transaction<'_>, view: &AnalysisView) -> Result<Vec<Window>, String> {
    // Sweep physical segments, effective transcript assignments and saved audio
    // coverage together. Work grows with interval count, not all interval pairs.
    type Event = (i64, u8, String, i32);
    let mut tracks: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    let mut interval = |track: String, start: i64, end: i64, kind: u8, speaker: String| {
        let events = tracks.entry(track).or_default();
        events.push((start, kind, speaker.clone(), 1));
        events.push((end, kind, speaker, -1));
    };
    let mut q=tx.prepare("SELECT COALESCE((SELECT a.track_id FROM meeting_audio_commits a WHERE a.meeting_id=s.meeting_id),s.track_id),COALESCE(speaker_id,''),start_us,end_us FROM speaker_segments s WHERE analysis_revision_id=?1").map_err(|e|e.to_string())?;
    let mut rows = q.query([&view.id]).map_err(|e| e.to_string())?;
    let mut count = 0;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > 100_000 {
            return Err("声の登録区間が上限を超えています".into());
        }
        interval(
            r.get(0).map_err(|e| e.to_string())?,
            r.get(2).map_err(|e| e.to_string())?,
            r.get(3).map_err(|e| e.to_string())?,
            0,
            r.get(1).map_err(|e| e.to_string())?,
        );
    }
    for u in &view.utterances {
        if u.speaker_ids.is_empty() {
            interval(u.track_id.clone(), u.start_us, u.end_us, 1, String::new());
        }
        for speaker in &u.speaker_ids {
            interval(u.track_id.clone(), u.start_us, u.end_us, 1, speaker.clone());
        }
    }
    let mut q=tx.prepare("SELECT c.track_id,c.start_us,c.end_us FROM audio_chunks c JOIN file_assets f ON f.id=c.file_asset_id WHERE c.meeting_id=?1 AND c.state='ready' AND f.state='ready'").map_err(|e|e.to_string())?;
    let mut rows = q.query([&view.meeting_id]).map_err(|e| e.to_string())?;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        interval(
            r.get(0).map_err(|e| e.to_string())?,
            r.get(1).map_err(|e| e.to_string())?,
            r.get(2).map_err(|e| e.to_string())?,
            3,
            String::new(),
        );
    }
    let mut q=tx.prepare("SELECT g.track_id,g.start_us,g.end_us FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id WHERE t.meeting_id=?1 AND g.recovered=0").map_err(|e|e.to_string())?;
    let mut rows = q.query([&view.meeting_id]).map_err(|e| e.to_string())?;
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        interval(
            r.get(0).map_err(|e| e.to_string())?,
            r.get(1).map_err(|e| e.to_string())?,
            r.get(2).map_err(|e| e.to_string())?,
            2,
            String::new(),
        );
    }
    let mut spans: Vec<Window> = vec![];
    for (track, mut events) in tracks {
        events.sort();
        let mut active = [BTreeMap::<String, i32>::new(), BTreeMap::new()];
        let mut gaps = 0;
        let mut coverage = 0;
        let mut previous = 0;
        for (time, kind, speaker, delta) in events {
            if time > previous
                && gaps == 0
                && coverage > 0
                && active[0].len() == 1
                && active[1].len() == 1
            {
                let physical = active[0].keys().next().unwrap();
                let effective = active[1].keys().next().unwrap();
                if !physical.is_empty() && physical == effective {
                    if let Some(last) = spans.last_mut().filter(|w| {
                        w.track_id == track && w.speaker_id == *physical && w.end_us == previous
                    }) {
                        last.end_us = time;
                    } else {
                        spans.push(Window {
                            track_id: track.clone(),
                            speaker_id: physical.clone(),
                            start_us: previous,
                            end_us: time,
                        });
                    }
                }
            }
            if kind < 2 {
                let values = &mut active[kind as usize];
                let n = values.get(&speaker).copied().unwrap_or(0) + delta;
                if n == 0 {
                    values.remove(&speaker);
                } else {
                    values.insert(speaker, n);
                }
            } else if kind == 2 {
                gaps += delta;
            } else {
                coverage += delta;
            }
            previous = time;
        }
    }
    let mut result = vec![];
    for speaker in &view.speakers {
        let mut selected = spans
            .iter()
            .filter(|w| w.speaker_id == speaker.id && w.end_us - w.start_us >= 1_000_000)
            .cloned()
            .collect::<Vec<_>>();
        selected.sort_by_key(|w| {
            (
                std::cmp::Reverse(w.end_us - w.start_us),
                w.start_us,
                w.track_id.clone(),
            )
        });
        for mut w in selected.into_iter().take(2) {
            let length = (w.end_us - w.start_us).min(30_000_000);
            w.start_us += (w.end_us - w.start_us - length) / 2;
            w.end_us = w.start_us + length;
            result.push(w);
        }
    }
    Ok(result)
}

impl Storage {
    pub(crate) fn has_voice_matching(
        &self,
        analysis: &str,
        book: &str,
        model: &str,
    ) -> Result<bool, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let options = super::voice_preferences::load(&db)?.payload;
        db.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE kind='voice_matching' AND input_analysis_id=?1 AND state NOT IN ('failed','cancelled') AND json_extract(parameters_json,'$.voice.bookHash')=?2 AND json_extract(parameters_json,'$.voice.matchingOptions.similarityThresholdMilli')=?3 AND json_extract(parameters_json,'$.voice.matchingOptions.marginThresholdMilli')=?4 AND json_extract(parameters_json,'$.voice.engine.embeddingModel.sha256')=?5)",params![analysis,book,options.similarity_threshold_milli,options.margin_threshold_milli,model],|r|r.get(0)).map_err(|e|e.to_string())
    }
    pub(crate) fn prepare_voice_sample_audio_cancellable(
        &self,
        sample_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::playback::PreparedAudio, String> {
        if let Some(audio) = self.saved_voice_playback(sample_id, cancelled)? {
            return Ok(audio);
        }
        let resolve = || -> Result<(String, String, i64, i64), String> {
            if cancelled() {
                return Err("音声の再生準備を取り消しました".into());
            }
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let source: Option<(Option<String>, Option<String>, i64, i64)> = db.query_row(
                "SELECT s.source_meeting_id,s.source_track_id,s.start_us,s.end_us FROM voice_samples s JOIN voice_people p ON p.id=s.person_id WHERE s.id=?1",
                [sample_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
            ).optional().map_err(|e| e.to_string())?;
            let (meeting, track, start, end) = source.ok_or("登録した声が見つかりません")?;
            Ok((
                meeting.ok_or("登録元の音声がありません")?,
                track.ok_or("登録元の音声トラックがありません")?,
                start,
                end,
            ))
        };
        // Resolve the saved sample, never a caller-supplied path, track or range.
        // It remains playable after its original analysis is replaced.
        let before = resolve()?;
        let audio = self.prepare_preview_track_audio_cancellable(
            &before.0, &before.1, before.2, before.3, cancelled,
        )?;
        if resolve()? != before {
            return Err("音声準備中に登録した声の参照先が変更されました".into());
        }
        Ok(audio)
    }

    pub(crate) fn voice_people(&self) -> Result<Vec<Person>, String> {
        people(
            &*self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?,
        )
    }
    pub(crate) fn voice_context(&self, meeting: &str) -> Result<Context, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let (view, _, working_hash) = working(&tx, meeting)?;
        Ok(Context {
            supported: cfg!(any(target_os = "macos", windows)),
            analysis_id: view.id.clone(),
            working_hash,
            book_hash: book_hash(&tx)?,
            people: people(&tx)?,
            matches: matches(&tx, &view.id)?,
            windows: windows(&tx, &view)?,
            registration_candidates: enrollment::candidates(&tx, &view)?,
        })
    }
    pub(crate) fn manage_voice_person(
        &self,
        id: &str,
        expected_version: i64,
        name: Option<String>,
        sample_id: Option<String>,
    ) -> Result<PersonMutation, String> {
        self.require_data_writable()?;
        let _voice_lock = self.lock_voice_audio()?;
        self.recover_voice_audio()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if tx
            .query_row("SELECT version FROM voice_people WHERE id=?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
            .map_err(|e| e.to_string())?
            != Some(expected_version)
        {
            return Err("人物登録が変更されています。再読込してください".into());
        }
        if sample_id.is_some() || name.is_none() {
            self.journal_voice_deletion(&tx, id, sample_id.as_deref())?;
        }
        if let Some(sample) = sample_id {
            if tx
                .execute(
                    "DELETE FROM voice_samples WHERE id=?1 AND person_id=?2",
                    params![sample, id],
                )
                .map_err(|e| e.to_string())?
                != 1
            {
                return Err("登録した声が見つかりません".into());
            }
            tx.execute(
                "UPDATE voice_people SET version=version+1 WHERE id=?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
        } else if let Some(name) = name {
            let name = valid_name(&name)?;
            tx.execute(
                "UPDATE voice_people SET name=?1,version=version+1 WHERE id=?2",
                params![name, id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute("DELETE FROM voice_people WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        let error = self.recover_voice_audio().err();
        Ok(PersonMutation {
            audio_cleanup_pending: error.is_some(),
            audio_cleanup_error: error,
        })
    }
    pub(crate) fn start_voice(&self, request: Start) -> Result<JobSnapshot, String> {
        self.start_voice_observed(request, false, None)
    }
    pub(crate) fn start_automatic_voice(&self, request: Start) -> Result<JobSnapshot, String> {
        self.start_voice_observed(request, true, None)
    }
    pub(crate) fn start_automatic_voice_intent(
        &self,
        request: Start,
        intent: &post_analysis::Intent,
    ) -> Result<JobSnapshot, String> {
        self.start_voice_observed(request, true, Some(intent))
    }
    fn start_voice_observed(
        &self,
        request: Start,
        automatic: bool,
        intent: Option<&post_analysis::Intent>,
    ) -> Result<JobSnapshot, String> {
        self.require_data_writable()?;
        Uuid::parse_str(&request.operation_id).map_err(|_| "声の処理の操作IDが不正です")?;
        // Read-back of an uncertain submission must not create a second person/job.
        {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            if let Some(id) = db
                .query_row(
                    "SELECT id FROM processing_jobs WHERE operation_id=?1",
                    [&request.operation_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
            {
                let (spec, _) = jobs::read_fixed_input(&db, &id)?;
                if spec.parameters["voiceRequest"]
                    != serde_json::to_value(&request).map_err(|e| e.to_string())?
                {
                    return Err("同じ操作IDの声の処理条件が異なります".into());
                }
                return jobs::load_job(&db, &id)?.ok_or("声の処理がありません".into());
            }
        }
        let (engine, extractor, managed, runtime) = if let Some(intent) = intent {
            let fixed = intent.execution()?;
            fixed.engine.verify(&|| false)?;
            (
                fixed.engine.clone(),
                fixed.extractor.clone(),
                fixed.managed.clone(),
                fixed.runtime.clone(),
            )
        } else {
            let engine = voice::engine(self)?;
            let extractor = crate::diarization::embedding::ExtractorConfig::capture(
                &crate::diarization::known::helper_path()?,
                &engine,
            )?;
            let managed = self.prepare_model_selection(
                "standard",
                model_profiles::jobs::Execution::Sherpa(&engine),
                &|| false,
            )?;
            let runtime = crate::local_engine::resident::prepare(self.root(), "sherpa")?;
            (engine, extractor, managed, runtime)
        };
        let source = self.recorded_analysis_input(&request.meeting_id)?.0;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (view, _, hash) = working(&tx, &request.meeting_id)?;
        if let Some(intent) = intent {
            intent.check(&tx, &view.id, &hash)?;
        }
        if view.id != request.analysis_id
            || hash != request.working_hash
            || book_hash(&tx)? != request.book_hash
        {
            return Err("会議または人物登録が変更されています。再確認してください".into());
        }
        let registration = request.speaker_id.is_some();
        if !registration && request.selected_windows.is_some() {
            return Err("照合には登録区間を指定できません".into());
        }
        if !registration && (request.person_id.is_some() || request.person_name.is_some()) {
            return Err("照合には登録する人物を指定できません".into());
        }
        if registration {
            if let Some(person) = &request.person_id {
                if !people(&tx)?.iter().any(|p| &p.id == person) || request.person_name.is_some() {
                    return Err("追加登録する人物の指定が不正です".into());
                }
            } else {
                valid_name(request.person_name.as_deref().unwrap_or(""))?;
            }
        }
        let selected: Vec<_> = if let Some(selected) = &request.selected_windows {
            enrollment::validate_selection(
                &enrollment::candidates(&tx, &view)?,
                request
                    .speaker_id
                    .as_deref()
                    .ok_or("登録する話者がありません")?,
                selected,
            )?;
            selected.clone()
        } else {
            windows(&tx, &view)?
                .into_iter()
                .filter(|w| {
                    request
                        .speaker_id
                        .as_ref()
                        .is_none_or(|s| s == &w.speaker_id)
                })
                .collect()
        };
        if selected.is_empty() {
            return Err(
                "登録・照合に使える単独の発言がありません。話者分離の結果と音声を確認してください"
                    .into(),
            );
        }
        let plan = Plan {
            resident_runtime: runtime,
            matching_options: if registration {
                None
            } else {
                Some(if let Some(intent) = intent {
                    intent.policy.matching
                } else {
                    super::voice_preferences::load(&tx)?.payload
                })
            },
            matching_policy: (!registration).then(voice::MatchingPolicy::fixed),
            schema_version: crate::current_formats::VOICE,
            engine,
            extractor,
            book_hash: request.book_hash.clone(),
            working_hash: hash,
            audio_manifest_hash: source.manifest_hash,
            person_id: request.person_id.clone(),
            person_name: request.person_name.clone(),
            windows: selected,
        };
        let spec = JobSpec {
            meeting_id: request.meeting_id.clone(),
            kind: if registration {
                JobKind::VoiceRegistration
            } else {
                JobKind::VoiceMatching
            },
            operation_id: request.operation_id.clone(),
            input_analysis_id: Some(view.id),
            input_hash: view.content_hash,
            profile_snapshot: model_profiles::jobs::attach(
                serde_json::json!({"profile":"standard","engine":"sherpa-onnx"}),
                managed,
            )?,
            parameters: serde_json::json!({"voice":plan,"voiceRequest":request,"automaticAnalysisProgress":automatic}),
            total_units: Some(1),
        };
        let job = jobs::enqueue_in_transaction(&tx, &spec)?;
        if let Some(intent) = intent {
            intent.enqueued(&tx, &job.id, false)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(job)
    }
    pub(crate) fn check_voice_plan(&self, spec: &JobSpec, plan: &Plan) -> Result<(), String> {
        self.require_data_writable()?;
        if self
            .recorded_analysis_input(&spec.meeting_id)?
            .0
            .manifest_hash
            != plan.audio_manifest_hash
        {
            return Err("声の処理に使用する音声が変更されています".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        check_plan(&tx, spec, plan)
    }
    pub(crate) fn publish_voice_result(
        &self,
        spec: &JobSpec,
        plan: &Plan,
        features: Vec<(Window, Vec<f32>)>,
        job_id: &str,
    ) -> Result<JobSnapshot, String> {
        self.require_data_writable()?;
        let _voice_lock = self.lock_voice_audio()?;
        self.recover_voice_audio()?;
        let _clip_cleanup = audio::Cleanup(self);
        let registration_person = plan
            .person_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut clips = Vec::new();
        if spec.kind == JobKind::VoiceRegistration {
            self.check_voice_plan(spec, plan)?;
            for (window, vector) in &features {
                voice::normalize(vector)?;
                if !plan.windows.contains(window) {
                    return Err("登録音声の区間が一致しません".into());
                }
                let id = Uuid::new_v4().to_string();
                clips.push(self.stage_voice_clip(
                    &registration_person,
                    &id,
                    &spec.meeting_id,
                    &window.track_id,
                    window.start_us,
                    window.end_us,
                    &|| self.get_job(job_id).map_or(true, |j| j.state != "running"),
                )?);
            }
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        check_plan(&tx, spec, plan)?;
        let job = jobs::load_job(&tx, job_id)?.ok_or("声の処理がありません")?;
        if job.state != "running" {
            return Err("声の処理が停止しています".into());
        }
        if jobs::read_fixed_input(&tx, job_id)?.0 != *spec {
            return Err("声の処理の固定ジョブが一致しません".into());
        }
        let mut seen = BTreeSet::new();
        for (window, vector) in &features {
            if !plan.windows.iter().any(|w| {
                w.speaker_id == window.speaker_id
                    && w.track_id == window.track_id
                    && w.start_us == window.start_us
                    && w.end_us == window.end_us
            }) || !seen.insert((
                window.speaker_id.clone(),
                window.track_id.clone(),
                window.start_us,
                window.end_us,
            )) {
                return Err("抽出した声の音声区間が固定条件と一致しません".into());
            }
            voice::normalize(vector)?;
        }
        if spec.kind == JobKind::VoiceRegistration {
            if features.is_empty() {
                return Err("この音声から声の特徴を取得できませんでした".into());
            }
            let person = registration_person;
            if plan.person_id.is_none() {
                if people(&tx)?.len() >= 1000 {
                    return Err("登録した人物は1000人までです".into());
                }
                tx.execute(
                    "INSERT INTO voice_people(id,name,created_at) VALUES(?1,?2,?3)",
                    params![
                        person,
                        valid_name(plan.person_name.as_deref().unwrap_or(""))?,
                        now_ms()?
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
            let count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM voice_samples WHERE person_id=?1",
                    [&person],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if count + features.len() as i64 > 64 {
                return Err("一人につき登録した声は64区間までです".into());
            }
            for ((window, vector), clip) in features.into_iter().zip(&clips) {
                tx.execute("INSERT INTO voice_samples(id,person_id,model_hash,extractor_version,dimension,vector,source_meeting_id,source_analysis_id,source_track_id,start_us,end_us,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![clip.id,person,plan.engine.embedding_model.sha256,voice::EXTRACTOR,vector.len() as i64,voice::encode(&vector)?,spec.meeting_id,spec.input_analysis_id,window.track_id,window.start_us,window.end_us,now_ms()?]).map_err(|e|e.to_string())?;
            }
            for clip in &clips {
                clip.insert(&tx)?;
            }
            tx.execute(
                "UPDATE voice_people SET version=version+1 WHERE id=?1",
                [&person],
            )
            .map_err(|e| e.to_string())?;
            validate_registry(&tx)?;
            let speaker = &plan
                .windows
                .first()
                .ok_or("登録する話者がありません")?
                .speaker_id;
            let name = people(&tx)?
                .into_iter()
                .find(|p| p.id == person)
                .ok_or("人物登録がありません")?
                .name;
            apply_names(
                &tx,
                &spec.meeting_id,
                spec.input_analysis_id
                    .as_deref()
                    .ok_or("解析版がありません")?,
                &[(speaker.clone(), Some(name.clone()))],
            )?;
            // A speaker added only in the working draft is not a row of the
            // immutable analysis. Save its name in that draft and its voice in
            // the independent registry, without inventing an analysis FK.
            let persisted: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM speakers WHERE analysis_revision_id=?1 AND id=?2)",
                    params![spec.input_analysis_id, speaker],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if persisted {
                tx.execute("INSERT INTO voice_matches(id,meeting_id,analysis_revision_id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at) VALUES(?1,?2,?3,?4,?5,?6,'confirmed',NULL,?7,'manual_enrollment_v1',?8) ON CONFLICT(analysis_revision_id,speaker_id) DO UPDATE SET person_id=excluded.person_id,name_snapshot=excluded.name_snapshot,state='confirmed',similarity=NULL,model_hash=excluded.model_hash,policy_version=excluded.policy_version,created_at=excluded.created_at",params![Uuid::new_v4().to_string(),spec.meeting_id,spec.input_analysis_id,speaker,person,name,plan.engine.embedding_model.sha256,now_ms()?]).map_err(|e|e.to_string())?;
            }
        } else {
            publish_matches_observed(&tx, spec, plan, features, &|done, total| {
                if let Ok(mut map) = self.execution_progress.lock() {
                    map.insert(
                        job_id.into(),
                        ExecutionProgress {
                            phase: if done == total { "saving" } else { "matching" }.into(),
                            completed: plan.windows.len() + done,
                            total: plan.windows.len() + total + 1,
                            phase_completed: done,
                            phase_total: total,
                        },
                    );
                }
            })?;
        }
        for clip in &clips {
            tx.execute(
                "DELETE FROM voice_audio_operations WHERE sample_key=?1 AND kind='install'",
                [&clip.id],
            )
            .map_err(|e| e.to_string())?;
        }
        let checkpoint = "{\"voiceResult\":true}";
        let sequence: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM job_checkpoints WHERE job_id=?1",
                [job_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO job_checkpoints(id,job_id,sequence,completed_units,checkpoint_json,checkpoint_hash,created_at) VALUES(?1,?2,?3,1,?4,?5,?6)",params![Uuid::new_v4().to_string(),job_id,sequence,checkpoint,jobs::checkpoint_hash(&job.spec_hash,1,checkpoint),now_ms()?]).map_err(|e|e.to_string())?;
        // Side effects and completion are one transaction: no half-registered person.
        tx.execute("UPDATE processing_jobs SET state='succeeded',completed_units=1,finished_at=?1,updated_at=?1 WHERE id=?2 AND state='running'",params![now_ms()?,job_id]).map_err(|e|e.to_string())?;
        let result = jobs::load_job(&tx, job_id)?.ok_or("声の処理がありません")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub(crate) fn decide_voice_match(
        &self,
        meeting: &str,
        analysis: &str,
        speaker: &str,
        person: Option<String>,
        expected_working_hash: &str,
    ) -> Result<(), String> {
        self.require_data_writable()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (view, _, hash) = working(&tx, meeting)?;
        if view.id != analysis
            || hash != expected_working_hash
            || !view.speakers.iter().any(|s| s.id == speaker)
        {
            return Err("文字起こしが変更されています。再読込してください".into());
        }
        let existing = matches(&tx, analysis)?
            .into_iter()
            .find(|m| m.speaker_id == speaker)
            .ok_or("照合結果がありません")?;
        if let Some(person) = person {
            let name = people(&tx)?
                .into_iter()
                .find(|p| p.id == person)
                .ok_or("人物登録がありません")?
                .name;
            apply_names(
                &tx,
                meeting,
                analysis,
                &[(speaker.to_string(), Some(name.clone()))],
            )?;
            tx.execute("UPDATE voice_matches SET person_id=?1,name_snapshot=?2,state='confirmed',similarity=NULL WHERE id=?3",params![person,name,existing.id]).map_err(|e|e.to_string())?;
        } else {
            // Clear only a name that still matches this identity's applied snapshot.
            if matches!(existing.state.as_str(), "confirmed" | "automatic")
                && view.speakers.iter().any(|s| {
                    s.id == speaker && s.display_name.as_deref() == Some(&existing.name_snapshot)
                })
            {
                apply_names(&tx, meeting, analysis, &[(speaker.to_string(), None)])?;
            }
            tx.execute(
                "UPDATE voice_matches SET person_id=NULL,state='dismissed' WHERE id=?1",
                [existing.id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
}
fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        Err("人物名を1〜200文字で入力してください".into())
    } else {
        Ok(name.into())
    }
}
fn check_plan(tx: &Transaction<'_>, spec: &JobSpec, plan: &Plan) -> Result<(), String> {
    let (view, _, hash) = working(tx, &spec.meeting_id)?;
    if Some(&view.id) != spec.input_analysis_id.as_ref()
        || view.content_hash != spec.input_hash
        || hash != plan.working_hash
        || book_hash(tx)? != plan.book_hash
    {
        return Err("会議・編集・人物登録が変更されています。新しく照合してください".into());
    }
    if serde_json::to_value(plan).map_err(|e| e.to_string())? != spec.parameters["voice"] {
        return Err("声の処理の固定条件が一致しません".into());
    }
    if plan.schema_version != crate::current_formats::VOICE || plan.resident_runtime.is_none() {
        return Err("非対応の声の処理形式です".into());
    }
    if spec.kind == JobKind::VoiceMatching {
        plan.matching_options
            .ok_or("照合の閾値がありません")?
            .validate()?;
        if plan.matching_policy != Some(voice::MatchingPolicy::fixed()) {
            return Err("声の自動適用方針が不正です".into());
        }
    } else if plan.matching_options.is_some() || plan.matching_policy.is_some() {
        return Err("登録の計画に照合条件を混在させることはできません".into());
    }
    if spec.kind == JobKind::VoiceRegistration
        && spec.parameters["voiceRequest"]["selectedWindows"].is_array()
    {
        let request: Start = serde_json::from_value(spec.parameters["voiceRequest"].clone())
            .map_err(|e| e.to_string())?;
        if request.selected_windows.as_ref() != Some(&plan.windows) {
            return Err("選択した登録区間と固定条件が一致しません".into());
        }
        return enrollment::validate_selection(
            &enrollment::candidates(tx, &view)?,
            request
                .speaker_id
                .as_deref()
                .ok_or("登録する話者がありません")?,
            &plan.windows,
        );
    }
    if plan.schema_version != crate::current_formats::VOICE {
        return Err("声の処理の計画形式が不正です".into());
    }
    let allowed = windows(tx, &view)?;
    if plan.windows.iter().any(|w| {
        !allowed.iter().any(|a| {
            a.speaker_id == w.speaker_id
                && a.track_id == w.track_id
                && a.start_us == w.start_us
                && a.end_us == w.end_us
        })
    }) {
        return Err("声の音声区間が変更されています".into());
    }
    Ok(())
}
fn apply_names(
    tx: &Transaction<'_>,
    meeting: &str,
    analysis: &str,
    names: &[(String, Option<String>)],
) -> Result<(), String> {
    let mut draft = if let Some(d) = drafts::load_draft(tx, meeting)? {
        d
    } else {
        let id = Uuid::new_v4().to_string();
        let payload = TranscriptDraftPayload::default();
        let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        let hash = drafts::draft_hash(&json);
        tx.execute("INSERT INTO working_drafts(id,meeting_id,target_kind,base_analysis_id,schema_version,payload_json,content_hash,lock_version,active,created_at,updated_at) VALUES(?1,?2,'transcript',?3,4,?4,?5,0,1,?6,?6)",params![id,meeting,analysis,json,hash,now_ms()?]).map_err(|e|e.to_string())?;
        drafts::load_draft(tx, meeting)?.ok_or("下書きがありません")?
    };
    for (speaker, name) in names {
        draft
            .payload
            .speaker_names
            .retain(|e| &e.speaker_id != speaker);
        draft.payload.speaker_names.push(drafts::SpeakerNameEdit {
            speaker_id: speaker.clone(),
            display_name: name.clone(),
        });
    }
    drafts::save_in_transaction(tx, &draft.id, draft.lock_version, draft.payload)?;
    Ok(())
}
#[cfg(test)]
fn publish_matches(
    tx: &Transaction<'_>,
    spec: &JobSpec,
    plan: &Plan,
    features: Vec<(Window, Vec<f32>)>,
) -> Result<(), String> {
    publish_matches_observed(tx, spec, plan, features, &|_, _| {})
}
fn publish_matches_observed(
    tx: &Transaction<'_>,
    spec: &JobSpec,
    plan: &Plan,
    features: Vec<(Window, Vec<f32>)>,
    observe: &impl Fn(usize, usize),
) -> Result<(), String> {
    let (view, _, _) = working(tx, &spec.meeting_id)?;
    let old = matches(tx, &view.id)?;
    let mut q=tx.prepare("SELECT person_id,dimension,vector FROM voice_samples WHERE model_hash=?1 AND extractor_version=?2 ORDER BY person_id,id").map_err(|e|e.to_string())?;
    let mut rows = q
        .query(params![
            plan.engine.embedding_model.sha256,
            voice::EXTRACTOR
        ])
        .map_err(|e| e.to_string())?;
    let mut registered: BTreeMap<String, Vec<Vec<f32>>> = BTreeMap::new();
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        registered
            .entry(r.get(0).map_err(|e| e.to_string())?)
            .or_default()
            .push(voice::decode(
                &r.get::<_, Vec<u8>>(2).map_err(|e| e.to_string())?,
                r.get::<_, u32>(1).map_err(|e| e.to_string())? as usize,
            )?);
    }
    let mut centroids = BTreeMap::new();
    for (person, samples) in registered {
        if let Ok(c) = voice::centroid(&samples) {
            centroids.insert(person, c);
        }
    }
    let people = people(tx)?;
    let mut queries: BTreeMap<String, Vec<Vec<f32>>> = BTreeMap::new();
    let mut clean_speakers = BTreeSet::new();
    for (w, v) in features {
        if w.end_us - w.start_us
            >= plan
                .matching_policy
                .as_ref()
                .map_or(3_000_000, |p| p.min_query_duration_us)
        {
            clean_speakers.insert(w.speaker_id.clone());
        }
        queries.entry(w.speaker_id).or_default().push(v);
    }
    let mut ranked = Vec::new();
    let speakers = plan
        .windows
        .iter()
        .map(|w| w.speaker_id.clone())
        .collect::<BTreeSet<_>>();
    let total = speakers.len();
    observe(0, total);
    for (index, speaker) in speakers.into_iter().enumerate() {
        struct Observation<'a, F: Fn(usize, usize)>(&'a F, usize, usize);
        impl<F: Fn(usize, usize)> Drop for Observation<'_, F> {
            fn drop(&mut self) {
                (self.0)(self.1, self.2);
            }
        }
        let _observation = Observation(observe, index + 1, total);
        let vectors = queries.remove(&speaker).unwrap_or_default();
        if old.iter().any(|m| {
            m.speaker_id == speaker
                && (matches!(m.state.as_str(), "confirmed" | "dismissed")
                    || spec.kind == JobKind::VoiceMatching
                        && m.state == "automatic"
                        && view
                            .speakers
                            .iter()
                            .any(|s| s.id == speaker && s.display_name.is_some()))
        }) {
            continue;
        }
        let vector = match voice::centroid(&vectors) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mut scores = centroids
            .iter()
            .filter_map(|(id, c)| voice::similarity(&vector, c).ok().map(|s| (id, s)))
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        let Some((person, score)) = scores.first() else {
            continue;
        };
        let options = plan.matching_options.ok_or("声の照合設定がありません")?;
        let automatic = options.accepts(
            *score,
            scores.get(1).map(|s| s.1),
            clean_speakers.contains(&speaker),
        ) && view
            .speakers
            .iter()
            .any(|s| s.id == speaker && s.display_name.is_none());
        if !automatic && *score < 0.5 {
            continue;
        }
        ranked.push((speaker, (*person).clone(), *score, automatic));
    }
    // If distinct overlapping voices nominate the same identity, neither is automatic.
    let mut conflicts = BTreeSet::new();
    for (a, person, _, _) in &ranked {
        for (b, other, _, _) in &ranked {
            if a != b && person == other {
                let overlap:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM speaker_segments a JOIN speaker_segments b ON a.analysis_revision_id=b.analysis_revision_id AND a.track_id=b.track_id AND a.start_us<b.end_us AND b.start_us<a.end_us WHERE a.analysis_revision_id=?1 AND a.speaker_id=?2 AND b.speaker_id=?3)",params![view.id,a,b],|r|r.get(0)).map_err(|e|e.to_string())?;
                if overlap {
                    conflicts.insert(a.clone());
                    conflicts.insert(b.clone());
                }
            }
        }
    }
    for speaker in plan
        .windows
        .iter()
        .map(|w| &w.speaker_id)
        .collect::<BTreeSet<_>>()
    {
        tx.execute("DELETE FROM voice_matches WHERE analysis_revision_id=?1 AND speaker_id=?2 AND state='candidate'",params![view.id,speaker]).map_err(|e|e.to_string())?;
    }
    for (speaker, person, _, _) in &ranked {
        for existing in old.iter().filter(|m| {
            m.speaker_id != *speaker
                && m.person_id.as_ref() == Some(person)
                && matches!(m.state.as_str(), "confirmed" | "automatic")
        }) {
            let overlap:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM speaker_segments a JOIN speaker_segments b ON a.analysis_revision_id=b.analysis_revision_id AND a.track_id=b.track_id AND a.start_us<b.end_us AND b.start_us<a.end_us WHERE a.analysis_revision_id=?1 AND a.speaker_id=?2 AND b.speaker_id=?3)",params![view.id,speaker,existing.speaker_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if overlap {
                conflicts.insert(speaker.clone());
            }
        }
    }
    let policy_version = plan
        .matching_options
        .ok_or("声の照合設定がありません")?
        .policy_version();
    let mut names = Vec::new();
    for (speaker, person, score, automatic) in ranked {
        let name = people
            .iter()
            .find(|p| p.id == person)
            .ok_or("人物登録がありません")?
            .name
            .clone();
        let automatic = automatic && !conflicts.contains(&speaker);
        tx.execute("INSERT INTO voice_matches(id,meeting_id,analysis_revision_id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(analysis_revision_id,speaker_id) DO UPDATE SET person_id=excluded.person_id,name_snapshot=excluded.name_snapshot,state=excluded.state,similarity=excluded.similarity,model_hash=excluded.model_hash,policy_version=excluded.policy_version,created_at=excluded.created_at",params![Uuid::new_v4().to_string(),spec.meeting_id,view.id,speaker,person,name,if automatic{"automatic"}else{"candidate"},score,plan.engine.embedding_model.sha256,policy_version,now_ms()?]).map_err(|e|e.to_string())?;
        if automatic {
            names.push((speaker, Some(name)));
        }
    }
    if !names.is_empty() {
        apply_names(tx, &spec.meeting_id, &view.id, &names)?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::storage) mod tests {
    use super::*;
    pub(in crate::storage) fn fixture() -> (tempfile::TempDir, Storage, String, String) {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (m, _, _) = super::super::playback::tests::fixture(&s);
        let manifest_hash = s.canonical_audio_plan(&m).unwrap().manifest_hash;
        let input = AnalysisInput {
            audio_manifest_hash: manifest_hash,
            profile_snapshot: serde_json::json!({}),
            config: serde_json::json!({}),
            speakers: vec![SpeakerInput {
                key: "voice-a".into(),
                label: "話者A".into(),
            }],
            segments: vec![SpeakerSegmentInput {
                track_id: "track".into(),
                speaker_key: Some("voice-a".into()),
                start_us: 0,
                end_us: 1_000_000,
                overlap_group: None,
                confidence: None,
            }],
            utterances: vec![UtteranceInput {
                track_id: "track".into(),
                start_us: 0,
                end_us: 1_000_000,
                text: "登録用の発言".into(),
                speaker_keys: vec!["voice-a".into()],
                confidence: None,
            }],
        };
        let id = s.create_analysis_candidate(&m, input).unwrap();
        s.commit_analysis_candidate(
            &m,
            &id,
            s.get_meeting(&m).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
        let speaker = s.load_current_analysis(&m).unwrap().unwrap().speakers[0]
            .id
            .clone();
        (root, s, m, speaker)
    }
    pub(super) fn enqueue(
        s: &Storage,
        m: &str,
        speaker: &str,
        registration: bool,
    ) -> (JobSnapshot, JobSpec, Plan) {
        enqueue_selected(s, m, speaker, registration, None)
    }
    pub(super) fn enqueue_selected(
        s: &Storage,
        m: &str,
        speaker: &str,
        registration: bool,
        selected: Option<Vec<Window>>,
    ) -> (JobSnapshot, JobSpec, Plan) {
        let c = s.voice_context(m).unwrap();
        let fp = crate::local_engine::Fingerprint {
            path: PathBuf::from(crate::test_support::absolute("/test-only")),
            size: 1,
            sha256: "a".repeat(64),
        };
        let engine = crate::diarization::sherpa::SherpaConfig {
            executable: fp.clone(),
            segmentation_model: fp.clone(),
            embedding_model: fp.clone(),
            runtime_libraries: vec![fp.clone()],
            threads: 1,
            cluster_threshold_milli: 500,
            window_shift_milli: 100,
            min_duration_on_ms: 0,
            min_duration_off_ms: 0,
            speech_sensitivity_milli: 0,
        };
        let plan = Plan {
            resident_runtime: crate::local_engine::resident::prepare(s.root(), "sherpa").unwrap(),
            matching_options: (!registration).then(Default::default),
            matching_policy: (!registration).then(voice::MatchingPolicy::fixed),
            schema_version: crate::current_formats::VOICE,
            engine,
            extractor: crate::diarization::embedding::ExtractorConfig {
                library_index: 0,
                helper: fp.clone(),
                library: fp,
            },
            book_hash: c.book_hash,
            working_hash: c.working_hash,
            audio_manifest_hash: s.recorded_analysis_input(m).unwrap().0.manifest_hash,
            person_id: None,
            person_name: registration.then(|| "田中".into()),
            windows: selected.clone().unwrap_or_else(|| {
                c.windows
                    .into_iter()
                    .filter(|w| w.speaker_id == speaker)
                    .collect()
            }),
        };
        let view = s.load_current_analysis(m).unwrap().unwrap();
        let operation_id = Uuid::new_v4().to_string();
        let request = Start {
            meeting_id: m.into(),
            operation_id: operation_id.clone(),
            analysis_id: view.id.clone(),
            working_hash: plan.working_hash.clone(),
            book_hash: plan.book_hash.clone(),
            speaker_id: registration.then(|| speaker.into()),
            person_id: None,
            person_name: plan.person_name.clone(),
            selected_windows: selected,
        };
        let spec = JobSpec {
            meeting_id: m.into(),
            operation_id,
            kind: if registration {
                JobKind::VoiceRegistration
            } else {
                JobKind::VoiceMatching
            },
            input_analysis_id: Some(view.id),
            input_hash: view.content_hash,
            profile_snapshot: serde_json::json!({"profile":"test"}),
            parameters: serde_json::json!({"voice":plan,"voiceRequest":request}),
            total_units: Some(1),
        };
        let job = s.enqueue_job(spec.clone()).unwrap();
        assert_eq!(s.acquire_local_inference_job().unwrap().unwrap().id, job.id);
        (job, spec, plan)
    }
    pub(in crate::storage) fn register(s: &Storage, m: &str, speaker: &str) -> String {
        let (job, spec, plan) = enqueue(s, m, speaker, true);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        s.fixed_job_input(&job.id).unwrap();
        s.voice_people().unwrap()[0].id.clone()
    }
    pub(in crate::storage) fn seed_person(s: &Storage, m: &str) -> String {
        let person = Uuid::new_v4().to_string();
        let db = s.connection.lock().unwrap();
        db.execute("INSERT INTO voice_people VALUES(?1,'田中',1,0)", [&person])
            .unwrap();
        db.execute(
            "INSERT INTO voice_samples VALUES(?1,?2,?3,?4,2,?5,?6,NULL,'track',0,1000000,0)",
            params![
                Uuid::new_v4().to_string(),
                person,
                "a".repeat(64),
                voice::EXTRACTOR,
                voice::encode(&[1., 0.]).unwrap(),
                m
            ],
        )
        .unwrap();
        person
    }
    #[test]
    fn user_matching_plan_is_frozen_and_deduplication_tracks_model_registry_and_preferences() {
        let (_root, s, m, speaker) = fixture();
        let (_, mut spec, mut plan) = enqueue(&s, &m, &speaker, false);
        plan.schema_version = crate::current_formats::VOICE;
        plan.matching_options =
            Some(super::super::voice_preferences::VoiceMatchingPreferences::default());
        plan.matching_policy = Some(voice::MatchingPolicy::fixed());
        spec.operation_id = Uuid::new_v4().to_string();
        spec.parameters["voice"] = serde_json::to_value(&plan).unwrap();
        s.enqueue_job(spec.clone()).unwrap();
        let model = &plan.engine.embedding_model.sha256;
        assert!(s
            .has_voice_matching(&plan_source(&spec), &plan.book_hash, model)
            .unwrap());
        assert!(!s
            .has_voice_matching(&plan_source(&spec), &"f".repeat(64), model)
            .unwrap());
        assert!(!s
            .has_voice_matching(&plan_source(&spec), &plan.book_hash, &"e".repeat(64))
            .unwrap());
        s.save_voice_matching_preferences(
            0,
            super::super::voice_preferences::VoiceMatchingPreferences {
                similarity_threshold_milli: 700,
                margin_threshold_milli: 50,
            },
        )
        .unwrap();
        assert!(!s
            .has_voice_matching(&plan_source(&spec), &plan.book_hash, model)
            .unwrap());
        let mut db = s.connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        check_plan(&tx, &spec, &plan).unwrap(); // the old frozen thresholds still apply
        let mut invalid = plan.clone();
        invalid.matching_policy = None;
        let mut changed = spec.clone();
        changed.parameters["voice"] = serde_json::to_value(&invalid).unwrap();
        assert!(check_plan(&tx, &changed, &invalid).is_err());
        invalid = plan.clone();
        invalid.schema_version = 1;
        changed.parameters["voice"] = serde_json::to_value(&invalid).unwrap();
        assert!(check_plan(&tx, &changed, &invalid).is_err());
    }
    fn plan_source(spec: &JobSpec) -> String {
        spec.input_analysis_id.clone().unwrap()
    }

    #[test]
    fn user_thresholds_override_evaluated_policy_and_candidate_floor_but_require_clean_audio() {
        for (score, duration, threshold, expected) in [
            (0.9, 3_000_000, 800, "automatic"),
            (0.4, 3_000_000, 300, "automatic"),
            (0.9, 2_999_999, 800, "candidate"),
        ] {
            let (_root, s, m, speaker) = fixture();
            seed_person(&s, &m);
            let (_job, spec, mut plan) = enqueue(&s, &m, &speaker, false);
            plan.schema_version = crate::current_formats::VOICE;
            plan.matching_policy = Some(voice::MatchingPolicy::fixed());
            plan.matching_options =
                Some(super::super::voice_preferences::VoiceMatchingPreferences {
                    similarity_threshold_milli: threshold,
                    margin_threshold_milli: 100,
                });
            let mut window = plan.windows[0].clone();
            window.end_us = duration;
            let mut db = s.connection.lock().unwrap();
            let tx = db.transaction().unwrap();
            tx.execute(
                "INSERT INTO voice_policies VALUES('strict',?1,?2,0.99,0.99,?3,0)",
                params!["a".repeat(64), voice::EXTRACTOR, "b".repeat(64)],
            )
            .unwrap();
            publish_matches(
                &tx,
                &spec,
                &plan,
                vec![(window, vec![score, (1_f32 - score * score).sqrt()])],
            )
            .unwrap();
            let found = matches(&tx, spec.input_analysis_id.as_deref().unwrap()).unwrap();
            assert_eq!(found[0].state, expected);
            let policy: String = tx
                .query_row("SELECT policy_version FROM voice_matches", [], |r| r.get(0))
                .unwrap();
            assert!(policy.starts_with("user_thresholds_v1:"));
            if expected == "automatic" {
                assert_eq!(
                    drafts::load_draft(&tx, &m)
                        .unwrap()
                        .unwrap()
                        .payload
                        .speaker_names[0]
                        .display_name
                        .as_deref(),
                    Some("田中")
                );
            }
        }
    }

    #[test]
    fn voice_sample_playback_reads_only_the_registered_track_interval() {
        let (_root, s, m, _speaker) = fixture();
        seed_person(&s, &m);
        let sample = s.voice_people().unwrap()[0].samples[0].id.clone();
        s.connection.lock().unwrap().execute(
            "UPDATE voice_samples SET start_us=250000,end_us=750000,source_analysis_id='retired-analysis' WHERE id=?1", [&sample],
        ).unwrap();
        let before = s.voice_people().unwrap();
        let mut audio = s
            .prepare_voice_sample_audio_cancellable(&sample, &crate::test_support::never_cancelled)
            .unwrap();
        assert_eq!(
            (audio.start_us, audio.end_us, audio.total_frames()),
            (250000, 750000, 8000)
        );
        assert_eq!(audio.track_kind, "processed");
        let pcm = audio.read_pcm(0, 8000).unwrap();
        let values: Vec<_> = pcm
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert!(values[..4000].iter().all(|v| *v == 0.25));
        assert!(values[4000..].iter().all(|v| *v == -0.5));
        assert_eq!(
            serde_json::to_value(s.voice_people().unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }

    #[test]
    fn voice_sample_playback_rejects_deleted_sources_and_cancelled_or_unknown_samples() {
        let (_root, s, m, _speaker) = fixture();
        seed_person(&s, &m);
        let sample = s.voice_people().unwrap()[0].samples[0].id.clone();
        assert!(s
            .prepare_voice_sample_audio_cancellable(
                "unknown",
                &crate::test_support::never_cancelled
            )
            .is_err());
        assert!(s
            .prepare_voice_sample_audio_cancellable(&sample, &|| true)
            .err()
            .unwrap()
            .contains("取り消"));
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE file_assets SET state='missing'", [])
            .unwrap();
        assert!(s
            .prepare_voice_sample_audio_cancellable(&sample, &crate::test_support::never_cancelled)
            .is_err());
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE voice_samples SET source_meeting_id=NULL,source_track_id=NULL",
                [],
            )
            .unwrap();
        assert!(s
            .prepare_voice_sample_audio_cancellable(&sample, &crate::test_support::never_cancelled)
            .err()
            .unwrap()
            .contains("登録元"));
        assert_eq!(s.voice_people().unwrap()[0].samples.len(), 1);
    }

    #[test]
    fn voice_sample_playback_rechecks_registration_after_preparing_audio() {
        let (_root, s, m, _speaker) = fixture();
        seed_person(&s, &m);
        let sample = s.voice_people().unwrap()[0].samples[0].id.clone();
        let checks = std::cell::Cell::new(0);
        let error = s
            .prepare_voice_sample_audio_cancellable(&sample, &|| {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    s.connection
                        .lock()
                        .unwrap()
                        .execute("DELETE FROM voice_samples WHERE id=?1", [&sample])
                        .unwrap();
                }
                false
            })
            .err()
            .unwrap();
        assert!(error.contains("登録した声"), "{error}");
    }

    #[test]
    fn voice_sample_playback_rejects_foreign_tracks_and_changed_physical_audio() {
        let (root, s, m, _speaker) = fixture();
        seed_person(&s, &m);
        let sample = s.voice_people().unwrap()[0].samples[0].id.clone();
        let other = s.create_meeting("別の会議").unwrap();
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE voice_samples SET source_meeting_id=?1 WHERE id=?2",
                params![other.id, sample],
            )
            .unwrap();
        assert!(s
            .prepare_voice_sample_audio_cancellable(&sample, &crate::test_support::never_cancelled)
            .is_err());
        s.connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE voice_samples SET source_meeting_id=?1 WHERE id=?2",
                params![m, sample],
            )
            .unwrap();
        let path = root
            .path()
            .join("meetings")
            .join(m)
            .join("audio/meeting.wav");
        let mut bytes = std::fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(s
            .prepare_voice_sample_audio_cancellable(&sample, &crate::test_support::never_cancelled)
            .err()
            .unwrap()
            .contains("ハッシュ"));
    }

    #[test]
    fn registration_and_completion_are_atomic_and_reject_empty_and_cancelled_results() {
        let (_root, s, m, speaker) = fixture();
        let (job, spec, plan) = enqueue(&s, &m, &speaker, true);
        assert!(s
            .publish_voice_result(&spec, &plan, vec![], &job.id)
            .is_err());
        assert!(s.voice_people().unwrap().is_empty());
        assert_eq!(s.get_job(&job.id).unwrap().state, "running");
        s.request_job_pause(&job.id, "user").unwrap();
        s.save_job_checkpoint(&job.id, 0, 0, serde_json::json!({"voiceResult":false}))
            .unwrap();
        s.finish_job(&job.id, 0, "cancelled", None).unwrap();
        assert!(s
            .publish_voice_result(
                &spec,
                &plan,
                vec![(plan.windows[0].clone(), vec![1., 0.])],
                &job.id
            )
            .is_err());
        assert!(s.voice_people().unwrap().is_empty());
        register(&s, &m, &speaker);
        assert_eq!(s.voice_people().unwrap()[0].samples.len(), 1);
        assert_eq!(s.voice_context(&m).unwrap().matches[0].state, "confirmed");
        assert_eq!(
            s.load_transcript_draft(&m)
                .unwrap()
                .unwrap()
                .payload
                .speaker_names[0]
                .display_name
                .as_deref(),
            Some("田中")
        );
    }
    #[test]
    fn uncalibrated_match_is_a_candidate_and_manual_confirmation_uses_draft_history() {
        let (_root, s, m, speaker) = fixture();
        let person = seed_person(&s, &m);
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        let context = s.voice_context(&m).unwrap();
        assert_eq!(context.matches[0].state, "candidate");
        assert!(s.load_transcript_draft(&m).unwrap().is_none());
        s.decide_voice_match(
            &m,
            &context.analysis_id,
            &speaker,
            Some(person.clone()),
            &context.working_hash,
        )
        .unwrap();
        let draft = s.load_transcript_draft(&m).unwrap().unwrap();
        assert_eq!(
            draft.payload.speaker_names[0].display_name.as_deref(),
            Some("田中")
        );
        assert_eq!(draft.lock_version, 1);
        assert!(s
            .decide_voice_match(
                &m,
                &context.analysis_id,
                &speaker,
                None,
                &context.working_hash
            )
            .is_err());
        let context = s.voice_context(&m).unwrap();
        s.decide_voice_match(
            &m,
            &context.analysis_id,
            &speaker,
            None,
            &context.working_hash,
        )
        .unwrap();
        assert!(s
            .load_transcript_draft(&m)
            .unwrap()
            .unwrap()
            .payload
            .speaker_names[0]
            .display_name
            .is_none());
        assert_eq!(
            s.voice_people().unwrap()[0].samples.len(),
            1,
            "confirmation must never train the registry"
        );
    }
    #[test]
    fn model_mismatch_unknown_voice_and_registry_changes_cannot_silently_apply_names() {
        let (_root, s, m, speaker) = fixture();
        let person = seed_person(&s, &m);
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE voice_samples SET model_hash=?1", ["b".repeat(64)])
            .unwrap();
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        assert!(s.voice_context(&m).unwrap().matches.is_empty());
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        let p = s.voice_people().unwrap().remove(0);
        s.manage_voice_person(&person, p.version, Some("変更後".into()), None)
            .unwrap();
        assert!(s
            .publish_voice_result(
                &spec,
                &plan,
                vec![(plan.windows[0].clone(), vec![1., 0.])],
                &job.id
            )
            .is_err());
        assert!(s.voice_context(&m).unwrap().matches.is_empty());
    }
    #[test]
    fn removing_person_keeps_applied_name_and_removing_source_keeps_registered_features() {
        let (_root, s, m, speaker) = fixture();
        let person = seed_person(&s, &m);
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        let c = s.voice_context(&m).unwrap();
        s.decide_voice_match(
            &m,
            &c.analysis_id,
            &speaker,
            Some(person.clone()),
            &c.working_hash,
        )
        .unwrap();
        {
            let db = s.connection.lock().unwrap();
            db.execute("UPDATE file_assets SET state='missing'", [])
                .unwrap();
        }
        assert_eq!(s.voice_people().unwrap()[0].samples.len(), 1);
        assert!(!s.voice_people().unwrap()[0].samples[0].source_available);
        let p = s.voice_people().unwrap().remove(0);
        assert!(s
            .manage_voice_person(&person, p.version - 1, None, None)
            .is_err());
        s.manage_voice_person(&person, p.version, None, None)
            .unwrap();
        assert!(s.voice_people().unwrap().is_empty());
        assert_eq!(
            s.load_transcript_draft(&m)
                .unwrap()
                .unwrap()
                .payload
                .speaker_names[0]
                .display_name
                .as_deref(),
            Some("田中")
        );
        assert!(s.voice_context(&m).unwrap().matches[0].person_id.is_none());
    }
    #[test]
    fn short_queries_remain_candidates_despite_evaluated_policy() {
        let (_root, s, m, speaker) = fixture();
        seed_person(&s, &m);
        {
            let db = s.connection.lock().unwrap();
            db.execute(
                "INSERT INTO voice_policies VALUES(?1,?2,?3,0.9,0.1,?4,0)",
                params![
                    Uuid::new_v4().to_string(),
                    "a".repeat(64),
                    voice::EXTRACTOR,
                    "b".repeat(64)
                ],
            )
            .unwrap();
        }
        let before = s.load_current_analysis(&m).unwrap();
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        assert_eq!(s.voice_context(&m).unwrap().matches[0].state, "candidate");
        assert_eq!(s.load_current_analysis(&m).unwrap(), before);
        assert!(s.load_transcript_draft(&m).unwrap().is_none());
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![-1., 0.])],
            &job.id,
        )
        .unwrap();
        assert!(s.load_transcript_draft(&m).unwrap().is_none());
    }
    #[test]
    fn lost_acceptance_and_completion_replies_reuse_the_original_registration() {
        let (_root, s, m, speaker) = fixture();
        let (job, spec, plan) = enqueue(&s, &m, &speaker, true);
        let request: Start =
            serde_json::from_value(spec.parameters["voiceRequest"].clone()).unwrap();
        assert_eq!(s.start_voice(request.clone()).unwrap().id, job.id);
        let mut changed = request.clone();
        changed.person_name = Some("別の名前".into());
        assert!(s.start_voice(changed).is_err());
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        assert_eq!(s.start_voice(request).unwrap().state, "succeeded");
        assert_eq!(s.voice_people().unwrap().len(), 1);
        assert_eq!(s.voice_people().unwrap()[0].samples.len(), 1);
    }
    #[test]
    fn calibrated_policy_keeps_ambiguous_and_manually_named_speakers_as_candidates() {
        let (_root, s, m, speaker) = fixture();
        seed_person(&s, &m);
        seed_person(&s, &m);
        {
            let db = s.connection.lock().unwrap();
            db.execute(
                "INSERT INTO voice_policies VALUES(?1,?2,?3,0.9,0.1,?4,0)",
                params![
                    Uuid::new_v4().to_string(),
                    "a".repeat(64),
                    voice::EXTRACTOR,
                    "b".repeat(64)
                ],
            )
            .unwrap();
        }
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        assert_eq!(s.voice_context(&m).unwrap().matches[0].state, "candidate");
        assert!(s.load_transcript_draft(&m).unwrap().is_none());
        let mut draft = s
            .create_transcript_draft(&m, &spec.input_analysis_id.clone().unwrap())
            .unwrap();
        draft.payload.speaker_names.push(drafts::SpeakerNameEdit {
            speaker_id: speaker.clone(),
            display_name: Some("手動の名前".into()),
        });
        s.save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
            .unwrap();
        let (job, spec, plan) = enqueue(&s, &m, &speaker, false);
        s.publish_voice_result(
            &spec,
            &plan,
            vec![(plan.windows[0].clone(), vec![1., 0.])],
            &job.id,
        )
        .unwrap();
        assert_eq!(
            s.load_transcript_draft(&m)
                .unwrap()
                .unwrap()
                .payload
                .speaker_names[0]
                .display_name
                .as_deref(),
            Some("手動の名前")
        );
    }
}
