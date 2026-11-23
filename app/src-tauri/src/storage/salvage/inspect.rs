use super::contents::{Audio, Gap, MeetingContents, Person, Sample};
use super::salvage_schema::MeetingRecord;
use super::*;
use base64::Engine;
use rusqlite::types::FromSql;
use rusqlite::{Row, ToSql, Transaction};
use std::collections::BTreeSet;

pub(super) const SETTINGS: &[(&str, &str)] = &[
    ("ui_preferences", "表示設定"),
    ("minutes_prompt_preferences", "議事録の指示"),
    ("capture_microphone", "マイク選択"),
    ("whisper_profiles", "文字起こし設定"),
    ("llama_profiles", "議事録生成設定"),
    ("diarization_profiles", "話者分離設定"),
    ("voice_matching", "話者照合設定"),
    ("speaker_completion", "話者補完設定"),
];

pub(super) fn known_setting(key: &str) -> bool {
    SETTINGS.iter().any(|(name, _)| *name == key)
}

pub(super) fn validate_setting(db: &Connection, key: &str) -> Result<(), String> {
    match key {
        "ui_preferences" => ui_preferences::load(db).map(|_| ()),
        "minutes_prompt_preferences" => minutes_prompt_preferences::load(db).map(|_| ()),
        "capture_microphone" => capture_preferences::load(db).map(|_| ()),
        "whisper_profiles" => preferences::load(db).map(|_| ()),
        "llama_profiles" => minutes_preferences::load(db).map(|_| ()),
        "diarization_profiles" => diarization_preferences::load(db).map(|_| ()),
        "voice_matching" => voice_preferences::load(db).map(|_| ()),
        "speaker_completion" => completion_preferences::load(db).map(|_| ()),
        _ => Err("未対応の設定です".into()),
    }
}

fn detach_setting(db: &Connection, setting: &mut Setting) -> Result<bool, String> {
    let (json, changed) = match setting.key.as_str() {
        "whisper_profiles" => {
            let mut payload = preferences::load(db)?.payload;
            let before = payload.clone();
            for engine in [&mut payload.standard, &mut payload.high_accuracy] {
                engine.executable_path.clear();
                engine.model_path.clear();
                engine.vad_model_path = None;
                engine.backend = None;
            }
            (serde_json::to_string(&payload), payload != before)
        }
        "llama_profiles" => {
            let mut payload = minutes_preferences::load(db)?.payload;
            let before = payload.clone();
            for engine in [&mut payload.standard, &mut payload.high_accuracy] {
                engine.executable_path.clear();
                engine.model_path.clear();
            }
            (serde_json::to_string(&payload), payload != before)
        }
        "diarization_profiles" => {
            let mut payload = diarization_preferences::load(db)?.payload;
            let before = payload.clone();
            for engine in [&mut payload.standard, &mut payload.high_accuracy] {
                engine.executable_path.clear();
                engine.segmentation_model_path.clear();
                engine.embedding_model_path.clear();
                engine.runtime_library_paths.clear();
            }
            (serde_json::to_string(&payload), payload != before)
        }
        _ => return Ok(false),
    };
    if changed {
        setting.json = json.map_err(err)?;
        setting.hash = mon_core::hash::sha256_hex(setting.json.as_bytes());
    }
    Ok(changed)
}

fn short(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(450)
        .collect()
}

fn recover_cell<T: FromSql>(
    row: &Row<'_>,
    index: usize,
    valid: impl FnOnce(&T) -> bool,
    fallback: T,
    incomplete: &mut bool,
) -> T {
    match row.get(index) {
        Ok(value) if valid(&value) => value,
        _ => {
            *incomplete = true;
            fallback
        }
    }
}

fn scan(
    db: &Connection,
    sql: &str,
    args: &[&dyn ToSql],
    cancelled: &impl Fn() -> bool,
    mut row: impl FnMut(&Row<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let mut q = db.prepare(sql).map_err(err)?;
    let mut rows = q.query(args).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        check(cancelled)?;
        row(r)?;
    }
    Ok(())
}

struct Inspection<'a, F> {
    plan: Plan,
    stage: Directory,
    model_stage: crate::model_placement::fs::Directory,
    data_root: &'a Path,
    checked_bytes: u64,
    capacity_error: Option<String>,
    cancelled: &'a F,
}

impl<F: Fn() -> bool> Inspection<'_, F> {
    fn require_stage_capacity(&mut self, size: u64) -> Result<(), String> {
        capacity(&self.stage.path, add_bytes(size, RESERVE)?).map_err(|error| {
            self.capacity_error = Some(error.clone());
            error
        })
    }

    fn add(
        &mut self,
        kind: &str,
        source_id: Option<&str>,
        label: &str,
        status: Status,
        reason: Option<&str>,
    ) {
        self.plan.preview.items.push(Item {
            kind: kind.into(),
            source_id: source_id.map(short),
            label: short(label),
            status,
            reason: reason.map(short),
        });
    }

    fn part<T>(
        &mut self,
        kind: &str,
        id: Option<&str>,
        label: &str,
        value: Result<Option<T>, String>,
        notes: &mut BTreeMap<String, String>,
    ) -> Option<T> {
        match value {
            Ok(Some(item)) => {
                self.add(kind, id, label, Status::Recoverable, None);
                Some(item)
            }
            Ok(None) => {
                self.add(kind, id, label, Status::Absent, None);
                None
            }
            Err(reason) => {
                self.add(kind, id, label, Status::Unavailable, Some(&reason));
                notes.insert(kind.into(), short(&reason));
                None
            }
        }
    }

    fn save_audio(
        &mut self,
        owner: &str,
        relative: &str,
        asset: &Asset,
        frames: i64,
        voice: bool,
        _registered: bool,
    ) -> Result<(), String> {
        if self.plan.assets.contains_key(&asset.id) || !id(&asset.id) {
            return Err("音声ファイルを特定できないため、復元できません".into());
        }
        let size = u64::try_from(asset.size).map_err(err)?;
        // Open the source and check path, layout and frame count before
        // creating a staged file. Revalidate the SHA during the actual copy.
        let mut input = crate::model_inspection::Source::open(self.data_root, relative)?;
        if voice {
            let clip = crate::audio::VerifiedAudio::open_cancellable(
                input.file.try_clone().map_err(err)?,
                size,
                &asset.hash,
                self.cancelled,
            )?;
            if !clip.format.is_voice_clip()
                || i64::try_from(clip.format.frames).map_err(err)? != frames
            {
                return Err("登録音声の形式・長さが不正です".into());
            }
        } else {
            contents::verify_meeting_wav(&mut input.file, size, frames)?;
        }
        input.verify()?;
        #[cfg(any(target_os = "macos", windows))]
        let source_target = (_registered
            && super::super::deletion::salvage_source::managed_audio(relative))
        .then(|| super::super::deletion::salvage_source::Target::capture(self.data_root, relative))
        .and_then(Result::ok)
        .flatten();
        self.require_stage_capacity(size)?;
        copy_checked(
            self.data_root,
            relative,
            &self.stage,
            &asset.id,
            size,
            &asset.hash,
            self.cancelled,
        )?;
        #[cfg(any(target_os = "macos", windows))]
        let source_target = source_target.filter(|expected| {
            input.verify().is_ok()
                && expected.proof.size == size
                && super::super::deletion::salvage_source::Target::capture(self.data_root, relative)
                    .is_ok_and(|now| now.as_ref() == Some(expected))
        });
        self.checked_bytes = add_bytes(self.checked_bytes, size)?;
        self.plan.assets.insert(
            asset.id.clone(),
            Staged {
                owner: owner.into(),
                voice,
                asset: asset.clone(),
                #[cfg(any(target_os = "macos", windows))]
                source_target,
            },
        );
        Ok(())
    }

    fn audio(&mut self, db: &Connection, meeting: &Meeting) -> Result<Option<Audio>, String> {
        let ((track, asset, frames), independent) = match meeting_audio::read(db, &meeting.id) {
            Ok(Some(plan)) => {
                let track = plan.tracks.first().ok_or("共通会議音声がありません")?;
                let chunk = track
                    .chunks
                    .first()
                    .ok_or("共通会議音声の区間がありません")?;
                (
                    (
                        track.id.clone(),
                        Asset {
                            id: chunk.asset_id.clone(),
                            size: chunk.size,
                            hash: chunk.hash.clone(),
                        },
                        chunk.frames,
                    ),
                    false,
                )
            }
            result if matches!(meeting.lifecycle.as_str(), "recorded" | "imported") => {
                // A verified meeting-owned PCM can still be copied when its
                // canonical publication or imported-WAV promotion was interrupted.
                let fallback = independent_wav(db, &meeting.id)?.ok_or_else(|| {
                    result
                        .err()
                        .unwrap_or_else(|| "この会議の音声ファイルを確認できませんでした".into())
                })?;
                (fallback, true)
            }
            Ok(None) if meeting.source_type.is_none() && meeting.lifecycle != "recording" => {
                return Ok(None)
            }
            Ok(None) => return Err("録音や取り込みが完了した音声を確認できませんでした".into()),
            Err(error) => return Err(error),
        };
        let relative: String = db.query_row(
            "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2 AND media_type='audio/wav' AND state='ready'",
            params![asset.id, meeting.id], |r| r.get(0),
        ).map_err(err)?;
        if !relative.starts_with(&format!("meetings/{}/", meeting.id)) {
            return Err("この会議の音声ファイルと確認できませんでした".into());
        }
        let mut gaps = Vec::new();
        let mut broken_gaps = false;
        let scanned = scan(db, "SELECT id,start_us,end_us,reason_code,recovered FROM audio_gaps WHERE track_id=?1 ORDER BY start_us,id",
            &[&track], self.cancelled, |r| {
                let parse = || -> Result<Gap, rusqlite::Error> {
                    Ok(Gap { id:r.get(0)?, start:r.get(1)?, end:r.get(2)?, reason:r.get(3)?, recovered:r.get(4)? })
                };
                if let Ok(gap) = parse() {
                    if gap.start >= 0 && gap.end > gap.start && id(&gap.id) {
                        gaps.push(gap);
                        return Ok(());
                    }
                }
                broken_gaps = true;
                Ok(())
            });
        if broken_gaps || scanned.is_err() {
            self.add(
                "audio",
                Some(&meeting.id),
                &meeting.title,
                Status::Uninspected,
                Some("音声が途切れた位置の一部を引き継げません"),
            );
        }
        self.save_audio(&meeting.id, &relative, &asset, frames, false, !independent)?;
        self.add(
            "audio",
            Some(&meeting.id),
            &meeting.title,
            Status::Recoverable,
            independent.then_some("確認できた音声ファイルを新しい音声として登録します"),
        );
        Ok(Some(Audio {
            asset,
            track,
            chunk: Uuid::new_v4().to_string(),
            frames,
            deleted: false,
            gaps,
        }))
    }

    fn audio_was_deleted(&self, db: &Connection, meeting: &str) -> Result<bool, String> {
        if let Some(record) = salvage_schema::meeting(db, meeting)? {
            if let Some(reason) = record.audio_unavailable {
                return Err(reason);
            }
            if record.audio_deleted {
                return Ok(true);
            }
        }
        deletion::audio::salvage_deleted(db, meeting, self.data_root, self.cancelled)
    }

    fn meeting(&mut self, tx: &Transaction<'_>, r: &Row<'_>) -> Result<(), String> {
        let identity = r.get::<_, String>(0).ok();
        let Some(identity) = identity.filter(|value| id(value)) else {
            self.add(
                "meeting",
                None,
                "会議情報",
                Status::Unavailable,
                Some("会議を特定できません"),
            );
            return Ok(());
        };
        let mut incomplete = false;
        let mut meeting = Meeting {
            id: identity,
            title: recover_cell(
                r,
                1,
                |v: &String| (1..=200).contains(&v.chars().count()) && !v.contains('\0'),
                "会議名を確認できません".into(),
                &mut incomplete,
            ),
            lifecycle: recover_cell(
                r,
                2,
                |v: &String| {
                    matches!(
                        v.as_str(),
                        "draft" | "recording" | "recorded" | "imported" | "archived"
                    )
                },
                "archived".into(),
                &mut incomplete,
            ),
            source_type: recover_cell(
                r,
                3,
                |v: &Option<String>| {
                    matches!(v.as_deref(), None | Some("recording" | "audio_import"))
                },
                None,
                &mut incomplete,
            ),
            created_at: recover_cell(r, 4, |v| *v >= 0, 0, &mut incomplete),
            updated_at: recover_cell(r, 5, |v| *v >= 0, 0, &mut incomplete),
            lock_version: 0,
        };
        let started_at = recover_cell(
            r,
            6,
            |v: &Option<i64>| v.is_none_or(|t| t >= 0),
            None,
            &mut incomplete,
        );
        let duration = recover_cell(r, 7, |v| *v >= 0, 0_i64, &mut incomplete);
        let key = &meeting.id;
        let label = &meeting.title;
        let mut notes = BTreeMap::new();
        let committed = r.get::<_, Option<String>>(8).map_err(err).and_then(|current| current.as_deref().map(|head| {
            let hash: String = tx.query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id=?1 AND meeting_id=?2 AND publication_state='committed'",
                params![head,key], |r| r.get(0),
            ).map_err(err)?;
            if revisions::analysis_content_hash(tx, head)? != hash {
                return Err("保存した文字起こしの内容を確認できませんでした".into());
            }
            let view = analysis::read_stored_analysis_content(tx, key, head, &hash)?;
            if view.utterances.len() > 100_000 || view.speakers.len() > 100 {
                return Err("文字起こしの件数が上限を超えています".into());
            }
            Ok(view)
        }).transpose());
        let transcript = match committed {
            Ok(Some(mut view)) => {
                // The copied DB remains query-only. A verified draft can
                // contribute edits to the new baseline; it is never copied as
                // a draft row or as an outstanding problem on the new meeting.
                let draft = drafts::load_draft(tx, key).and_then(|draft| {
                    draft
                        .map(|draft| {
                            if draft.base_analysis_id != view.id {
                                return Err("下書きの基底と元本文が一致しません".into());
                            }
                            drafts::validate_payload(tx, &view.id, draft.payload)
                        })
                        .transpose()
                });
                let body_only =
                    "保存済みの文字起こしを引き継ぎます。編集途中の下書きは復元できません";
                let reason = match draft {
                    Ok(None) => None,
                    Err(_) => Some(body_only),
                    Ok(Some(payload)) if payload == TranscriptDraftPayload::default() => None,
                    Ok(Some(payload)) => {
                        let mut edited = view.clone();
                        minutes_input::apply_edits(&mut edited, &payload);
                        if edited.utterances.len() > 100_000 || edited.speakers.len() > 100 {
                            Some(body_only)
                        } else {
                            view = edited;
                            Some("確認できた文字起こしの編集内容を本文に反映します")
                        }
                    }
                };
                self.add("transcript", Some(key), label, Status::Recoverable, reason);
                Some(view)
            }
            value => self.part("transcript", Some(key), label, value, &mut notes),
        };
        let markdown = self.part(
            "minutes",
            Some(key),
            label,
            minutes_markdown::read(tx, key).map(|doc| doc.map(|v| v.markdown)),
            &mut notes,
        );
        let vocabulary = self
            .part(
                "vocabulary",
                Some(key),
                label,
                contents::vocabulary(tx, &format!("meeting:{key}")).map(present_vec),
                &mut notes,
            )
            .unwrap_or_default();
        let status = self.audio_was_deleted(tx, key);
        let audio_deleted = status.as_ref().is_ok_and(|deleted| *deleted);
        let (audio, audio_unavailable) = match status.and_then(|deleted| {
            if deleted {
                self.add("audio", Some(key), label, Status::Deleted, None);
                Ok(None)
            } else {
                self.audio(tx, &meeting)
            }
        }) {
            Ok(Some(audio)) => (Some(audio), None),
            Ok(None) if !audio_deleted => {
                self.add("audio", Some(key), label, Status::Absent, None);
                (None, None)
            }
            Ok(None) => (None, None),
            Err(error) => {
                check(self.cancelled)?;
                self.add("audio", Some(key), label, Status::Unavailable, Some(&error));
                (None, Some(short(&error)))
            }
        };

        let metadata_notice =
            incomplete.then_some("一部の会議情報を確認できず、確認用の会議名や既定値で登録します");
        if let Some(reason) = metadata_notice {
            notes.insert("meeting".into(), reason.into());
        }
        let record = MeetingRecord {
            operation_id: self.plan.preview.operation_id.clone(),
            source_meeting_id: key.clone(),
            audio_unavailable,
            audio_deleted,
            notes,
        };
        let track = audio
            .as_ref()
            .map(|a| a.track.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.add(
            "meeting",
            Some(key),
            label,
            if incomplete {
                Status::Partial
            } else {
                Status::Recoverable
            },
            metadata_notice,
        );
        if meeting.lifecycle == "recording" {
            meeting.lifecycle = "archived".into();
        }
        let manifest = Asset {
            id: Uuid::new_v4().to_string(),
            size: contents::manifest_bytes().len() as i64,
            hash: mon_core::hash::sha256_hex(contents::manifest_bytes()),
        };
        self.plan.contents.meetings.push(MeetingContents {
            meeting,
            started_at,
            duration,
            source_id: Uuid::new_v4().to_string(),
            operation_id: Uuid::new_v4().to_string(),
            manifest,
            audio,
            transcript,
            markdown,
            vocabulary,
            track,
            manual_assignment_lineages: Vec::new(),
            segments: Vec::new(),
            matches: Vec::new(),
            salvage: Some(record),
        });
        Ok(())
    }

    fn people(&mut self, db: &Connection) -> Result<(), String> {
        let mut dimensions = BTreeMap::new();
        let mut samples = 0usize;
        scan(
            db,
            "SELECT id,name,created_at FROM voice_people NOT INDEXED ORDER BY rowid",
            &[],
            self.cancelled,
            |r| {
                let parse = || -> Result<Person, String> {
                    let id_value: String = r.get(0).map_err(err)?;
                    let name: String = r.get(1).map_err(err)?;
                    voice::valid_name(&name)?;
                    if !id(&id_value) {
                        return Err("登録した人物を特定できませんでした".into());
                    }
                    Ok(Person {
                        id: id_value,
                        name,
                        created_at: r.get(2).map_err(err)?,
                        samples: Vec::new(),
                    })
                };
                let mut person = match parse() {
                    Ok(person) => person,
                    Err(reason) => {
                        self.add(
                            "voice_person",
                            None,
                            "登録した人物",
                            Status::Unavailable,
                            Some(&reason),
                        );
                        return Ok(());
                    }
                };
                let sample_scan = scan(db,
                "SELECT id,model_hash,extractor_version,dimension,vector,source_meeting_id,source_analysis_id,source_track_id,start_us,end_us,created_at FROM voice_samples WHERE person_id=?1 ORDER BY id",
                &[&person.id], self.cancelled, |r| {
                    let sample = self.sample(r);
                    match sample {
                        Ok((mut sample, mut note)) => {
                            let model = (sample.model.clone(), sample.extractor.clone());
                            if samples >= 8192 || person.samples.len() >= 64
                                || dimensions.get(&model).is_some_and(|dim| *dim != sample.dimension)
                            {
                                self.add("voice_sample", Some(&sample.id), &person.name, Status::Unavailable,
                                    Some("登録件数の上限を超えたか、対応していない形式のため、この声は引き継げません"));
                                return Ok(());
                            }
                            let clip = self.clip(db, &person.id, &sample.id);
                            let status = match clip {
                                Ok(Some((asset, frames))) => {
                                    sample.audio = Some(asset);
                                    sample.frames = Some(frames);
                                    Status::Recoverable
                                }
                                outcome => {
                                    check(self.cancelled)?;
                                    let reason = match outcome {
                                        Err(reason) => reason,
                                        _ => "元の音声クリップがありません".into(),
                                    };
                                    note.push(format!("話者を見分けるための声の登録を引き継ぎます。{reason}"));
                                    Status::Partial
                                }
                            };
                            dimensions.insert(model, sample.dimension);
                            self.add("voice_sample", Some(&sample.id), &person.name, status,
                                (!note.is_empty()).then(|| note.join(" ")).as_deref());
                            person.samples.push(sample);
                            samples += 1;
                        }
                        Err(reason) => {
                            let sid = r.get::<_,String>(0).ok();
                            self.add("voice_sample", sid.as_deref(), &person.name, Status::Unavailable, Some(&reason));
                        }
                    }
                    Ok(())
                });
                if let Err(error) = sample_scan {
                    check(self.cancelled)?;
                    self.add(
                        "voice_sample",
                        None,
                        &person.name,
                        Status::Uninspected,
                        Some(&error),
                    );
                }
                self.add(
                    "voice_person",
                    Some(&person.id),
                    &person.name,
                    Status::Recoverable,
                    None,
                );
                self.plan.contents.people.push(person);
                Ok(())
            },
        )
    }

    fn sample(&self, r: &Row<'_>) -> Result<(Sample, Vec<String>), String> {
        let sid: String = r.get(0).map_err(err)?;
        let model: String = r.get(1).map_err(err)?;
        let extractor: String = r.get(2).map_err(err)?;
        let dimension: i64 = r.get(3).map_err(err)?;
        let vector: Vec<u8> = r.get(4).map_err(err)?;
        let start: i64 = r.get(8).map_err(err)?;
        let end: i64 = r.get(9).map_err(err)?;
        if !id(&sid)
            || !jobs::valid_hash(&model)
            || extractor != crate::voice::EXTRACTOR
            || start < 0
            || end <= start
        {
            return Err("登録した声のサンプルを読み取れませんでした".into());
        }
        crate::voice::decode(&vector, usize::try_from(dimension).map_err(err)?)?;
        let original: Option<String> = r.get(5).map_err(err)?;
        let source_analysis: Option<String> = r.get(6).map_err(err)?;
        let _source_track: Option<String> = r.get(7).map_err(err)?;
        let owner = self
            .plan
            .contents
            .meetings
            .iter()
            .find(|m| Some(m.meeting.id.as_str()) == original.as_deref());
        let note = original
            .as_ref()
            .map(|_| {
                owner.map_or_else(
                    || "元の会議との関連は引き継げません".into(),
                    |meeting| {
                        format!(
                            "元の会議「{}」との関連を引き継ぎます",
                            meeting.meeting.title
                        )
                    },
                )
            })
            .into_iter()
            .collect();
        Ok((
            Sample {
                id: sid,
                model,
                extractor,
                dimension,
                vector: base64::engine::general_purpose::STANDARD.encode(vector),
                source_meeting: owner.map(|m| m.meeting.id.clone()),
                source_analysis: owner
                    .and_then(|m| m.transcript.as_ref())
                    .filter(|a| Some(a.id.as_str()) == source_analysis.as_deref())
                    .map(|a| a.id.clone()),
                source_track: owner
                    .and_then(|m| m.audio.as_ref())
                    .map(|a| a.track.clone()),
                start,
                end,
                created_at: r.get(10).map_err(err)?,
                audio: None,
                frames: None,
            },
            note,
        ))
    }

    fn clip(
        &mut self,
        db: &Connection,
        person: &str,
        sample: &str,
    ) -> Result<Option<(Asset, i64)>, String> {
        let row: Option<(String, i64, String, i64)> = db.query_row(
            "SELECT relative_path,size_bytes,content_hash,frame_count FROM voice_sample_audio WHERE id=?1 AND state='ready'",
            [sample], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional().map_err(err)?;
        let Some((relative, size, hash, frames)) = row else {
            return Ok(None);
        };
        if relative != voice::audio::relative(person, sample)? {
            return Err("登録音声の所属が一致しません".into());
        }
        let asset = Asset {
            id: sample.into(),
            size,
            hash,
        };
        self.save_audio(person, &relative, &asset, frames, true, true)?;
        Ok(Some((asset, frames)))
    }

    fn settings(&mut self, db: &Connection) -> Result<(), String> {
        for &(key, label) in SETTINGS {
            check(self.cancelled)?;
            let result = db.query_row(
                "SELECT CASE WHEN length(CAST(value_json AS BLOB))<=262144 THEN value_json END,content_hash,lock_version,updated_at FROM app_settings WHERE key=?1",
                [key], |r| Ok(Setting { key: key.into(), json:r.get(0)?, hash:r.get(1)?, version:r.get(2)?, updated_at:r.get(3)? }),
            ).optional().map_err(err).and_then(|row| {
                row.map(|mut setting| {
                    if !(1..=9_007_199_254_740_991).contains(&setting.version) || setting.updated_at < 0 {
                        return Err("保存した設定を確認できませんでした".into());
                    }
                    validate_setting(db, key)?;
                    let detached = detach_setting(db, &mut setting)?;
                    Ok((setting, detached))
                }).transpose()
            });
            match result {
                Ok(Some((setting, detached))) => {
                    self.plan.settings.push(setting);
                    self.add(
                        "setting",
                        Some(key),
                        label,
                        if detached {
                            Status::Defaulted
                        } else {
                            Status::Recoverable
                        },
                        detached.then_some(
                            "実行に使うプログラムとモデルは選び直してください。ほかの設定は引き継ぎます",
                        ),
                    );
                }
                Ok(None) => self.add("setting", Some(key), label, Status::Absent, None),
                Err(error) => {
                    self.add("setting", Some(key), label, Status::Defaulted, Some(&error))
                }
            }
        }
        for (table, label, why) in [
            (
                "model_profiles",
                "使用モデルの選択",
                "新しい登録の互換性確認後に、使うモデルを選び直してください",
            ),
            (
                "voice_policies",
                "自動話者識別の評価条件",
                "元のモデルに対する評価結果を引き継ぎません",
            ),
        ] {
            match db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
                r.get::<_, bool>(0)
            }) {
                Ok(true) => self.add("setting", Some(table), label, Status::Defaulted, Some(why)),
                Ok(false) => self.add("setting", Some(table), label, Status::Absent, None),
                Err(error) => self.add(
                    "setting",
                    Some(table),
                    label,
                    Status::Uninspected,
                    Some(&err(error)),
                ),
            }
        }
        Ok(())
    }

    fn models(&mut self, db: &Connection, root: Option<&Path>) -> Result<(), String> {
        let root = root
            .map(|path| path.canonicalize().map_err(err))
            .transpose();
        let mut registrations = BTreeSet::new();
        scan(db, "SELECT id,model_name,manifest_json,manifest_hash FROM model_packages NOT INDEXED ORDER BY rowid", &[], self.cancelled, |r| {
            let label = r.get::<_,String>(1).unwrap_or_else(|_| "モデル".into());
            let old = r.get::<_,String>(0).ok();
            let result = (|| -> Result<PlacementRecord, String> {
                let old = old.as_deref().filter(|id_value| id(id_value)).ok_or("モデルを特定できませんでした")?;
                let root = root.as_ref().map_err(Clone::clone)?.as_ref().ok_or("元のモデル保存先が不明です")?;
                let anchor = crate::model_placement::fs::Directory::open_existing(root)?;
                let source = crate::model_placement::fs::Directory::open_existing(&anchor.path().join(old))?;
                let json: String = r.get(2).map_err(err)?;
                let hash: String = r.get(3).map_err(err)?;
                if json.len() > 65_536 || mon_core::hash::sha256_hex(json.as_bytes()) != hash {
                    return Err("モデルの登録情報を確認できません".into());
                }
                let mut record: PlacementRecord = serde_json::from_str(&json).map_err(err)?;
                record.validate()?;
                if record.package_id != old {
                    return Err("モデルの所属が一致しません".into());
                }
                let report = crate::model_inspection::inspect(source.path(), self.cancelled)?;
                if report.review_hash != record.review_hash {
                    return Err("モデルのファイルが確認済みの内容と一致しません".into());
                }
                if registrations.contains(&report.review_hash) {
                    return Err("同じ内容のモデルは一つの新規登録にまとめます".into());
                }
                self.require_stage_capacity(add_bytes(report.total_bytes, 65536)?)?;
                record.package_id = remap(&self.plan.preview.operation_id, "model_packages", "", old);
                record.download = None;
                record.validate()?;
                let target = self.model_stage.child(&record.package_id, true)?;
                crate::model_placement::copy_package(source.path(), &report, &target, None, self.cancelled)?;
                anchor.verify()?;
                source.verify()?;
                self.checked_bytes = add_bytes(self.checked_bytes, add_bytes(report.total_bytes, 65536)?)?;
                registrations.insert(report.review_hash);
                Ok(record)
            })();
            match result {
                Ok(record) => {
                    self.plan.models.push(record);
                    self.add("model", old.as_deref(), &label, Status::Recoverable, Some("復元後、このモデルを使えるか確認してください"));
                }
                Err(error) => {
                    check(self.cancelled)?;
                    self.add("model", old.as_deref(), &label, Status::Unavailable, Some(&error));
                }
            }
            Ok(())
        })
    }
}

fn independent_wav(db: &Connection, meeting: &str) -> Result<Option<(String, Asset, i64)>, String> {
    let mut query = db.prepare("SELECT t.id,f.id,f.size_bytes,f.content_hash,c.frame_count
        FROM meetings m
        JOIN audio_tracks t ON t.meeting_id=m.id
        JOIN audio_chunks c ON c.meeting_id=m.id AND c.track_id=t.id
        JOIN file_assets f ON f.meeting_id=m.id AND f.id=c.file_asset_id
        WHERE m.id=?1 AND (
            (m.source_type='recording' AND m.lifecycle='recorded' AND t.kind='processed' AND t.session_id IS NOT NULL)
            OR (m.source_type='audio_import' AND m.lifecycle='imported' AND t.kind IN ('processed','imported') AND t.import_id IS NOT NULL)
        ) AND t.state='ready' AND t.format='wav' AND t.sample_rate=16000 AND t.channels=1
        AND c.state='ready' AND c.sequence=0 AND c.start_us=0 AND c.sample_rate=16000 AND c.channels=1
        AND f.media_type='audio/wav' AND f.state='ready' AND f.hash_algorithm='sha256'
        AND NOT EXISTS (SELECT 1 FROM audio_chunks other WHERE other.track_id=t.id AND other.id!=c.id)
        LIMIT 2").map_err(err)?;
    let mut rows = query.query([meeting]).map_err(err)?;
    let value = rows
        .next()
        .map_err(err)?
        .map(|row| -> Result<_, rusqlite::Error> {
            Ok((
                row.get(0)?,
                Asset {
                    id: row.get(1)?,
                    size: row.get(2)?,
                    hash: row.get(3)?,
                },
                row.get(4)?,
            ))
        })
        .transpose()
        .map_err(err)?;
    if rows.next().map_err(err)?.is_some() {
        return Err("会議のWAV候補が複数あり、正本を特定できません".into());
    }
    Ok(value.filter(|(_, _, frames): &(_, _, i64)| {
        *frames > 0 && frames.checked_mul(1_000_000).is_some()
    }))
}

fn present_vec<T>(items: Vec<T>) -> Option<Vec<T>> {
    (!items.is_empty()).then_some(items)
}

pub(crate) fn inspect(
    operation_id: &str,
    source_root: &Path,
    source_models: Option<&Path>,
    source_db_copy: &Path,
    workspace: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<Plan, String> {
    if !id(operation_id) {
        return Err("データ復元の操作IDが不正です".into());
    }
    check(cancelled)?;
    let workspace = workspace.canonicalize().map_err(err)?;
    let copy_path = source_db_copy.canonicalize().map_err(err)?;
    if !copy_path.starts_with(&workspace) {
        return Err("SQLiteの確認用コピーが復元用の一時領域外にあります".into());
    }
    let source_root = source_root.canonicalize().map_err(err)?;
    let db = Connection::open_with_flags(
        &copy_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(err)?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON;")
        .map_err(err)?;
    let plan = Plan {
        preview: Preview {
            operation_id: operation_id.into(),
            source_root: source_root.to_str().ok_or("復元元を表示できません")?.into(),
            required_bytes: RESERVE,
            meeting_total: None,
            items: Vec::new(),
            recoverable_count: 0,
        },
        source_models: source_models.and_then(Path::to_str).map(str::to_owned),
        contents: Contents {
            version: 1,
            meetings: Vec::new(),
            people: Vec::new(),
            vocabulary: Vec::new(),
        },
        models: Vec::new(),
        settings: Vec::new(),
        assets: BTreeMap::new(),
    };
    let workspace_dir = Directory::open(&workspace)?;
    let stage = workspace_dir.create_child("salvage-assets")?;
    let models = workspace_dir.create_child("salvage-models")?;
    let mut scan_state = Inspection {
        plan,
        stage,
        model_stage: crate::model_placement::fs::Directory::root(&models.path)?,
        data_root: &source_root,
        checked_bytes: 0,
        capacity_error: None,
        cancelled,
    };
    let tx = db.unchecked_transaction().map_err(err)?;
    let mut count = 0;
    crate::operation_observation::stage("inspecting_salvage_meetings", None, "meetings");
    let scanned = scan(&tx, "SELECT id,title,lifecycle,source_type,created_at,updated_at,started_at_utc_ms,duration_us,current_analysis_id FROM meetings NOT INDEXED ORDER BY rowid",
        &[], cancelled, |row| {
            count += 1;
            scan_state.meeting(&tx, row)?;
            crate::operation_observation::advance(1);
            Ok(())
        });
    check(cancelled)?;
    match scanned {
        Ok(()) => scan_state.plan.preview.meeting_total = Some(count as usize),
        Err(reason) => scan_state.add(
            "meeting",
            None,
            "残りの会議情報",
            Status::Uninspected,
            Some(&reason),
        ),
    }
    let words = contents::vocabulary(&tx, "global").map(present_vec);
    scan_state.plan.contents.vocabulary = scan_state
        .part("vocabulary", None, "共通用語", words, &mut BTreeMap::new())
        .unwrap_or_default();
    crate::operation_observation::stage("inspecting_salvage_voices", None, "items");
    if let Err(reason) = scan_state.people(&tx) {
        check(cancelled)?;
        scan_state.add(
            "voice_person",
            None,
            "残りの人物・サンプル",
            Status::Uninspected,
            Some(&reason),
        );
    }
    scan_state.settings(&tx)?;
    crate::operation_observation::stage("inspecting_salvage_models", None, "items");
    if let Err(reason) = scan_state.models(&tx, source_models) {
        check(cancelled)?;
        scan_state.add(
            "model",
            None,
            "残りのモデル",
            Status::Uninspected,
            Some(&reason),
        );
    }
    drop(tx);
    check(cancelled)?;
    if let Some(reason) = scan_state.capacity_error {
        return Err(reason);
    }
    scan_state.stage.verify()?;
    scan_state.model_stage.verify()?;
    crate::native_fs::sync_directory(&workspace)?;
    let mut plan = scan_state.plan;
    plan.preview.recoverable_count = plan.contents.meetings.len()
        + plan.contents.people.len()
        + plan.contents.vocabulary.len()
        + plan.models.len()
        + plan.settings.len();
    // Source size is a conservative allowance for the rebuilt DB; include
    // serialization overhead and durable write reserve independently of audio.
    let source_size = std::fs::metadata(&copy_path).map_err(err)?.len();
    plan.preview.required_bytes =
        add_bytes(add_bytes(scan_state.checked_bytes, source_size)?, RESERVE)?;
    Ok(plan)
}
