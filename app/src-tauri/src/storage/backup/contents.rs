//! Format 8 carries current, typed application content, never a table dump.
//! The same constructor builds a trusted source image and the remapped restore.
use super::*;
use crate::storage::{analysis::AnalysisView, vocabulary::VocabularyEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};

pub(super) const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Contents {
    version: u32,
    meetings: Vec<MeetingContents>,
    vocabulary: Vec<VocabularyEntry>,
    people: Vec<Person>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Asset {
    id: String,
    size: i64,
    hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Audio {
    asset: Asset,
    track: String,
    chunk: String,
    frames: i64,
    deleted: bool,
    gaps: Vec<Gap>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Gap {
    id: String,
    start: i64,
    end: i64,
    reason: String,
    recovered: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MeetingContents {
    meeting: Meeting,
    started_at: Option<i64>,
    duration: i64,
    source_id: String,
    operation_id: String,
    manifest: Asset,
    audio: Option<Audio>,
    transcript: Option<AnalysisView>,
    manual_assignment_lineages: Vec<String>,
    segments: Vec<Segment>,
    markdown: Option<String>,
    vocabulary: Vec<VocabularyEntry>,
    matches: Vec<Match>,
    // Even a document-only transcript needs a scoped track for timed utterances.
    track: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Segment {
    id: String,
    speaker: Option<String>,
    start: i64,
    end: i64,
    overlap: Option<String>,
    confidence: Option<f64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Match {
    id: String,
    speaker: String,
    person: Option<String>,
    name: String,
    state: String,
    similarity: Option<f64>,
    model: String,
    policy: String,
    created_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Person {
    id: String,
    name: String,
    created_at: i64,
    samples: Vec<Sample>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Sample {
    id: String,
    model: String,
    extractor: String,
    dimension: i64,
    vector: String,
    source_meeting: Option<String>,
    source_analysis: Option<String>,
    source_track: Option<String>,
    start: i64,
    end: i64,
    created_at: i64,
    audio: Option<Asset>,
    frames: Option<i64>,
}

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn vocabulary(db: &Connection, scope: &str) -> Result<Vec<VocabularyEntry>, String> {
    // Verify the current book, including its ordering and persisted hash.
    Ok(super::super::vocabulary::load(db, scope)?.0.payload)
}
pub(super) fn manifest_bytes() -> &'static [u8] {
    br#"{"version":1,"kind":"meeting_snapshot","audioPolicy":"meeting_wav_pcm16_16000_v1"}"#
}

fn verify_meeting_wav(file: &mut (impl Read + Seek), size: u64, frames: i64) -> Result<(), String> {
    // Canonical meeting WAVs use RIFF or RF64, depending on their length.
    let layout = crate::audio::wave::inspect(file, size)?;
    if i64::try_from(layout.frames).map_err(err)? != frames {
        return Err("バックアップ音声の形式・長さが不正です".into());
    }
    Ok(())
}

impl Contents {
    pub(super) fn capture(
        db: &mut Connection,
        stage: &Path,
        deleted: &deleted_audio::Native,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(Self, Vec<StagedAsset>), String> {
        use base64::Engine;
        let mut meetings = Vec::new();
        let mut manifests = Vec::new();
        let all = read_meetings(db)?;
        let total = all.len() as u64;
        for (position, meeting) in all.into_iter().enumerate() {
            check_cancelled(cancelled)?;
            crate::operation_observation::report(
                "collecting_meetings",
                position as u64,
                total,
                "meetings",
            );
            let (started_at, duration, current): (Option<i64>, i64, Option<String>) = db.query_row(
                "SELECT started_at_utc_ms,duration_us,current_analysis_id FROM meetings WHERE id=?1", [&meeting.id],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)?;
            let (transcript, manual_assignment_lineages) = if let Some(current) = current {
                let tx = db.unchecked_transaction().map_err(err)?;
                let transcript = super::super::voice::working(&tx, &meeting.id)?.0;
                let manual =
                    super::super::speaker_assignment_update::manual_lineages(&tx, &current)?;
                let lineages =
                    super::super::minutes_input::lineage_map(&tx, "utterances", &current)?;
                let mut markers: Vec<_> = transcript
                    .utterances
                    .iter()
                    .filter(|u| manual.contains(lineages.get(&u.id).unwrap_or(&u.id)))
                    .map(|u| u.id.clone())
                    .collect();
                markers.sort_unstable();
                (Some(transcript), markers)
            } else {
                (None, Vec::new())
            };
            let is_deleted = deleted.contains_meeting(&meeting.id);
            let plan = if is_deleted {
                super::super::meeting_audio::read_historical(db, &meeting.id)?
            } else {
                super::super::meeting_audio::read(db, &meeting.id)?
            };
            if matches!(meeting.lifecycle.as_str(), "recorded" | "imported")
                && plan.is_none()
                && !is_deleted
            {
                return Err(
                    "会議音声の保存が完了していません。保存・回復後にバックアップしてください"
                        .into(),
                );
            }
            let audio = plan.map(|p| -> Result<Audio,String> {
                let t = p.tracks.first().ok_or("共通会議音声がありません")?;
                let c = t.chunks.first().ok_or("共通会議音声の区間がありません")?;
                let mut q = db.prepare("SELECT id,start_us,end_us,reason_code,recovered FROM audio_gaps WHERE track_id=?1 ORDER BY start_us,id").map_err(err)?;
                let gaps = q.query_map([&t.id], |r| Ok(Gap{id:r.get(0)?,start:r.get(1)?,end:r.get(2)?,reason:r.get(3)?,recovered:r.get(4)?})).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
                Ok(Audio {asset:Asset{id:c.asset_id.clone(),size:c.size,hash:c.hash.clone()},track:t.id.clone(),chunk:id(),frames:c.frames,deleted:is_deleted,gaps})
            }).transpose()?;
            let bytes = manifest_bytes();
            let manifest = Asset {
                id: id(),
                size: bytes.len() as i64,
                hash: format!("{:x}", Sha256::digest(bytes)),
            };
            let key = id();
            let path = stage.join("assets").join(&key);
            private_file(&path)?.write_all(bytes).map_err(err)?;
            crate::native_fs::sync_file(&path)?;
            manifests.push(StagedAsset {
                asset: FileAsset {
                    id: manifest.id.clone(),
                    meeting_id: meeting.id.clone(),
                    relative_path: String::new(),
                    media_type: "application/json".into(),
                    size_bytes: manifest.size,
                    content_hash: manifest.hash.clone(),
                    state: "ready".into(),
                },
                key,
                path,
            });
            let speakers: BTreeSet<_> = transcript
                .as_ref()
                .map(|t| t.speakers.iter().map(|s| s.id.clone()).collect())
                .unwrap_or_default();
            let mut matches = Vec::new();
            let mut segments = Vec::new();
            if let Some(t) = &transcript {
                // Keep current acoustic evidence independently of edited
                // utterance assignments. Playback and enrollment use the
                // canonical meeting track, including for retired source tracks.
                let mut q = db.prepare("SELECT id,speaker_id,start_us,end_us,overlap_group_id,confidence FROM speaker_segments WHERE analysis_revision_id=?1 ORDER BY id").map_err(err)?;
                segments = q
                    .query_map([&t.id], |r| {
                        Ok(Segment {
                            id: r.get(0)?,
                            speaker: r.get(1)?,
                            start: r.get(2)?,
                            end: r.get(3)?,
                            overlap: r.get(4)?,
                            confidence: r.get(5)?,
                        })
                    })
                    .map_err(err)?
                    .collect::<Result<_, _>>()
                    .map_err(err)?;
                let mut q = db.prepare("SELECT id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at FROM voice_matches WHERE analysis_revision_id=?1 ORDER BY speaker_id").map_err(err)?;
                let rows = q
                    .query_map([&t.id], |r| {
                        Ok(Match {
                            id: r.get(0)?,
                            speaker: r.get(1)?,
                            person: r.get(2)?,
                            name: r.get(3)?,
                            state: r.get(4)?,
                            similarity: r.get(5)?,
                            model: r.get(6)?,
                            policy: r.get(7)?,
                            created_at: r.get(8)?,
                        })
                    })
                    .map_err(err)?;
                for row in rows {
                    let row = row.map_err(err)?;
                    if speakers.contains(&row.speaker) {
                        matches.push(row);
                    }
                }
            }
            let markdown =
                super::super::minutes_markdown::read(db, &meeting.id)?.map(|m| m.markdown);
            let vocabulary = vocabulary(db, &format!("meeting:{}", meeting.id))?;
            let track = audio.as_ref().map(|a| a.track.clone()).unwrap_or_else(id);
            meetings.push(MeetingContents {
                meeting,
                started_at,
                duration,
                source_id: id(),
                operation_id: id(),
                manifest,
                audio,
                transcript,
                manual_assignment_lineages,
                segments,
                markdown,
                vocabulary,
                matches,
                track,
            });
        }
        crate::operation_observation::report("collecting_meetings", total, total, "meetings");
        super::super::voice::validate_registry(db)?;
        let mut people = Vec::new();
        let mut q = db
            .prepare("SELECT id,name,created_at FROM voice_people ORDER BY id")
            .map_err(err)?;
        let mut rows = q.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            check_cancelled(cancelled)?;
            let mut person = Person {
                id: r.get(0).map_err(err)?,
                name: r.get(1).map_err(err)?,
                created_at: r.get(2).map_err(err)?,
                samples: Vec::new(),
            };
            let mut q=db.prepare("SELECT s.id,s.model_hash,s.extractor_version,s.dimension,s.vector,s.source_meeting_id,s.source_analysis_id,s.source_track_id,s.start_us,s.end_us,s.created_at,a.state,a.size_bytes,a.content_hash,a.frame_count FROM voice_samples s LEFT JOIN voice_sample_audio a ON a.id=s.id WHERE s.person_id=?1 ORDER BY s.id").map_err(err)?;
            let mut rows = q.query([&person.id]).map_err(err)?;
            while let Some(r) = rows.next().map_err(err)? {
                let state: String = r
                    .get::<_, Option<String>>(11)
                    .map_err(err)?
                    .ok_or("登録音声の保存情報がありません")?;
                if !matches!(state.as_str(), "ready" | "unavailable") {
                    return Err(
                        "登録音声の保存・回復が未完了です。ボイス一覧から再試行してください".into(),
                    );
                }
                let sid: String = r.get(0).map_err(err)?;
                let audio = if state == "ready" {
                    Some(Asset {
                        id: sid.clone(),
                        size: r.get(12).map_err(err)?,
                        hash: r.get(13).map_err(err)?,
                    })
                } else {
                    None
                };
                let mut sample = Sample {
                    id: sid,
                    model: r.get(1).map_err(err)?,
                    extractor: r.get(2).map_err(err)?,
                    dimension: r.get(3).map_err(err)?,
                    vector: base64::engine::general_purpose::STANDARD
                        .encode(r.get::<_, Vec<u8>>(4).map_err(err)?),
                    source_meeting: r.get(5).map_err(err)?,
                    source_analysis: r.get(6).map_err(err)?,
                    source_track: r.get(7).map_err(err)?,
                    start: r.get(8).map_err(err)?,
                    end: r.get(9).map_err(err)?,
                    created_at: r.get(10).map_err(err)?,
                    audio,
                    frames: r.get(14).map_err(err)?,
                };
                let owner = meetings
                    .iter()
                    .find(|m| Some(&m.meeting.id) == sample.source_meeting.as_ref());
                sample.source_analysis = owner
                    .and_then(|m| m.transcript.as_ref())
                    .filter(|a| Some(&a.id) == sample.source_analysis.as_ref())
                    .map(|a| a.id.clone());
                sample.source_track = owner
                    .and_then(|m| m.audio.as_ref())
                    .map(|a| a.track.clone());
                if owner.is_none() {
                    sample.source_meeting = None;
                }
                person.samples.push(sample);
            }
            people.push(person);
        }
        Ok((
            Self {
                version: 1,
                meetings,
                vocabulary: vocabulary(db, "global")?,
                people,
            },
            manifests,
        ))
    }

    pub(super) fn assets(&self) -> Vec<(String, String, bool)> {
        let mut result = Vec::new();
        for m in &self.meetings {
            if let Some(a) = &m.audio {
                result.push((a.asset.id.clone(), m.meeting.id.clone(), a.deleted));
            }
        }
        for p in &self.people {
            for s in &p.samples {
                if s.audio.is_some() {
                    result.push((s.id.clone(), String::new(), false));
                }
            }
        }
        result
    }

    pub(super) fn verify_audio(
        &self,
        assets: &BTreeMap<String, PathBuf>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let verify = |a: &Asset, frames: i64, voice_clip: bool| -> Result<(), String> {
            let path = assets
                .get(&a.id)
                .ok_or("バックアップに音声ファイルがありません")?;
            let mut file = File::open(path).map_err(err)?;
            let size = u64::try_from(a.size).map_err(err)?;
            if !voice_clip {
                verify_meeting_wav(&mut file, size, frames)?;
            }
            let audio =
                crate::audio::VerifiedAudio::open_cancellable(file, size, &a.hash, cancelled)?;
            if (voice_clip && !audio.format.is_voice_clip())
                || i64::try_from(audio.format.frames).map_err(err)? != frames
            {
                return Err("バックアップ音声の形式・長さが不正です".into());
            }
            Ok(())
        };
        for m in &self.meetings {
            if let Some(a) = &m.audio {
                if !a.deleted {
                    verify(&a.asset, a.frames, false)?;
                }
            }
        }
        for p in &self.people {
            for s in &p.samples {
                if let Some(a) = &s.audio {
                    if a.id != s.id {
                        return Err("登録音声の識別子が一致しません".into());
                    }
                    verify(a, s.frames.ok_or("登録音声の長さがありません")?, true)?;
                } else if s.frames.is_some() {
                    return Err("欠落した登録音声に長さがあります".into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn write_database(
        &self,
        path: &Path,
        identity: &impl Fn(&str, &str, &str) -> Result<String, String>,
        asset_path: &impl Fn(&str, &str) -> Result<String, String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Connection, String> {
        use base64::Engine;
        if self.version != 1 {
            return Err("未対応の会議バックアップ形式です".into());
        }
        for m in &self.meetings {
            if m.transcript
                .as_ref()
                .is_some_and(|t| t.meeting_id != m.meeting.id)
            {
                return Err("文字起こしの会議が一致しません".into());
            }
            if let Some(a) = &m.audio {
                if a.track != m.track || a.frames <= 0 {
                    return Err("会議音声の所属・長さが不正です".into());
                }
            }
        }
        private_file(path)?.sync_all().map_err(err)?;
        let mut db = portable::initialize(Connection::open(path).map_err(err)?)?;
        db.execute_batch("PRAGMA journal_mode=MEMORY; PRAGMA synchronous=FULL;")
            .map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        tx.execute_batch("PRAGMA defer_foreign_keys=ON")
            .map_err(err)?;
        write_book(&tx, None, &self.vocabulary, identity)?;
        for p in &self.people {
            check_cancelled(cancelled)?;
            let person = identity("voice_people", "", &p.id)?;
            tx.execute(
                "INSERT INTO voice_people(id,name,version,created_at) VALUES(?1,?2,1,?3)",
                params![person, p.name, p.created_at],
            )
            .map_err(err)?;
            for s in &p.samples {
                let sid = identity("voice_samples", "", &s.id)?;
                let owner = s.source_meeting.as_deref();
                let reference = |domain: &str, key: &Option<String>| {
                    key.as_deref()
                        .map(|k| {
                            identity(
                                domain,
                                if domain == "meetings" {
                                    ""
                                } else {
                                    owner.unwrap_or("")
                                },
                                k,
                            )
                        })
                        .transpose()
                };
                let vector = base64::engine::general_purpose::STANDARD
                    .decode(&s.vector)
                    .map_err(err)?;
                tx.execute("INSERT INTO voice_samples(id,person_id,model_hash,extractor_version,dimension,vector,source_meeting_id,source_analysis_id,source_track_id,start_us,end_us,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![sid,person,s.model,s.extractor,s.dimension,vector,reference("meetings",&s.source_meeting)?,reference("analysis_revisions",&s.source_analysis)?,reference("audio_tracks",&s.source_track)?,s.start,s.end,s.created_at]).map_err(err)?;
                if let Some(a) = &s.audio {
                    tx.execute("INSERT INTO voice_sample_audio(id,relative_path,size_bytes,content_hash,frame_count,state) VALUES(?1,?2,?3,?4,?5,'ready')",params![sid,asset_path("",&s.id)?,a.size,a.hash,s.frames]).map_err(err)?;
                } else {
                    tx.execute(
                        "INSERT INTO voice_sample_audio(id,state) VALUES(?1,'unavailable')",
                        [&sid],
                    )
                    .map_err(err)?;
                }
            }
        }
        let total = self.meetings.len() as u64;
        for (index, m) in self.meetings.iter().enumerate() {
            check_cancelled(cancelled)?;
            crate::operation_observation::report(
                "building_meetings",
                index as u64,
                total,
                "meetings",
            );
            let old = &m.meeting.id;
            let mid = identity("meetings", "", old)?;
            let get = |domain: &str, key: &str| identity(domain, old, key);
            let source = m.meeting.source_type.as_deref().unwrap_or("recording");
            let has_track = m.audio.is_some() || m.transcript.is_some();
            let has_source = m.meeting.source_type.is_some() || has_track;
            let source_id = if has_source {
                get(
                    if source == "recording" {
                        "recording_sessions"
                    } else {
                        "audio_imports"
                    },
                    &m.source_id,
                )?
            } else {
                m.source_id.clone()
            };
            // An import without saved audio has no commit operation to remap.
            let operation = if (has_source && source == "recording")
                || m.audio.is_some()
                || m.markdown.is_some()
            {
                get("operation", &m.operation_id)?
            } else {
                m.operation_id.clone()
            };
            let manifest = get("file_assets", &m.manifest.id)?;
            let track = if has_track {
                get("audio_tracks", &m.track)?
            } else {
                m.track.clone()
            };
            let analysis = m
                .transcript
                .as_ref()
                .map(|a| get("analysis_revisions", &a.id))
                .transpose()?;
            tx.execute("INSERT INTO meetings(id,title,started_at_utc_ms,duration_us,lifecycle,source_type,next_analysis_no,created_at,updated_at,lock_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,0)",params![mid,m.meeting.title,m.started_at,m.duration,m.meeting.lifecycle,if m.meeting.source_type.is_some()||analysis.is_some() {Some(source)} else {None},if analysis.is_some(){2}else{1},m.meeting.created_at,m.meeting.updated_at]).map_err(err)?;
            insert_asset(
                &tx,
                &mid,
                &manifest,
                &asset_path(old, &m.manifest.id)?,
                "application/json",
                &m.manifest,
                false,
            )?;
            if has_source {
                if source == "recording" {
                    tx.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,duration_us,audio_manifest_hash,last_committed_us,created_at,updated_at) VALUES(?1,?2,'saved','system','{}',?3,?4,?5,?4,?6,?6)",params![source_id,mid,operation,m.duration,m.manifest.hash,m.meeting.created_at]).map_err(err)?;
                } else if source == "audio_import" {
                    let a = m.audio.as_ref();
                    let aid = a.map(|a| get("file_assets", &a.asset.id)).transpose()?;
                    tx.execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,detected_format,duration_us,original_asset_id,converted_asset_id,created_at,updated_at) VALUES(?1,?2,'ready','backup.wav','wav',?3,?4,?4,?5,?5)",params![source_id,mid,m.duration,aid,m.meeting.created_at]).map_err(err)?;
                    if let Some(a) = a {
                        let info = crate::audio_import::AudioInfo {
                            schema_version: 1,
                            detected_format: "wav".into(),
                            codec: "pcm".into(),
                            sample_rate: 16000,
                            channels: 1,
                            frames: u64::try_from(a.frames).map_err(err)?,
                            duration_us: a
                                .frames
                                .checked_mul(1_000_000)
                                .ok_or("音声の長さが範囲外です")?
                                / 16000,
                        };
                        info.validate()?;
                        tx.execute("INSERT INTO audio_import_commits(import_id,meeting_id,operation_id,source_size,source_hash,info_json,manifest_asset_id,manifest_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![source_id,mid,operation,a.asset.size,a.asset.hash,serde_json::to_string(&info).map_err(err)?,manifest,m.manifest.hash]).map_err(err)?;
                    }
                } else {
                    return Err("会議の保存元が不正です".into());
                }
            }
            if has_track {
                tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,import_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'processed','wav',16000,1,?5,?6,?7,?7)",params![track,mid,if source=="recording"{Some(&source_id)}else{None},if source=="audio_import"{Some(&source_id)}else{None},if m.audio.as_ref().is_some_and(|a|!a.deleted){"ready"}else{"failed"},serde_json::json!({"policy":super::super::meeting_audio::POLICY}).to_string(),m.meeting.created_at]).map_err(err)?;
            }
            if let Some(a) = &m.audio {
                if a.frames <= 0 || a.frames.checked_mul(2).and_then(|n| {
                    n.checked_add(crate::audio::wave::header(a.frames as u64).ok()?.len() as i64)
                }) != Some(a.asset.size)
                {
                    return Err("会議音声のサイズ・長さが不正です".into());
                }
                let aid = get("file_assets", &a.asset.id)?;
                insert_asset(
                    &tx,
                    &mid,
                    &aid,
                    &asset_path(old, &a.asset.id)?,
                    "audio/wav",
                    &a.asset,
                    false,
                )?;
                tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,?5,?6,16000,1,?7,?8)",params![get("audio_chunks",&a.chunk)?,mid,track,aid,a.frames.checked_mul(1_000_000).ok_or("音声の長さが範囲外です")?/16000,a.frames,if a.deleted{"missing"}else{"ready"},m.meeting.created_at]).map_err(err)?;
                // The insert guard requires a ready asset even for a missing
                // chunk. Finish its unavailable state inside this private transaction.
                if a.deleted {
                    tx.execute("UPDATE file_assets SET state='missing' WHERE id=?1", [&aid])
                        .map_err(err)?;
                }
                for g in &a.gaps {
                    tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,?3,?4,?5,?6)",params![get("audio_gaps",&g.id)?,track,g.start,g.end,g.reason,g.recovered]).map_err(err)?;
                }
                tx.execute("INSERT INTO meeting_audio_commits(meeting_id,track_id,manifest_asset_id,manifest_hash,frames,source_json,cleanup_json,phase,created_at) VALUES(?1,?2,?3,?4,?5,'{}','[]','cleaned',?6)",params![mid,track,manifest,m.manifest.hash,a.frames,m.meeting.created_at]).map_err(err)?;
            }
            if let (Some(t), Some(analysis)) = (&m.transcript, analysis) {
                let mut config = serde_json::json!({"snapshotFormat":8});
                let mut ranges = t.review_ranges.clone();
                for r in &mut ranges {
                    r.track_id = track.clone();
                }
                config["reviewRanges"] = serde_json::to_value(ranges).map_err(err)?;
                config["sourceRecognitionLineages"] = serde_json::to_value(
                    t.utterances
                        .iter()
                        .filter(|u| u.recognition_is_source)
                        .map(|u| get("utterance_lineage", &u.id))
                        .collect::<Result<Vec<_>, _>>()?,
                )
                .map_err(err)?;
                let utterance_ids: BTreeSet<_> =
                    t.utterances.iter().map(|u| u.id.as_str()).collect();
                if m.manual_assignment_lineages
                    .windows(2)
                    .any(|w| w[0] >= w[1])
                    || m.manual_assignment_lineages
                        .iter()
                        .any(|id| !utterance_ids.contains(id.as_str()))
                {
                    return Err("バックアップの手動話者割り当てが不正です".into());
                }
                let mut manual = m
                    .manual_assignment_lineages
                    .iter()
                    .map(|id| get("utterance_lineage", id))
                    .collect::<Result<Vec<_>, _>>()?;
                manual.sort_unstable();
                config["manualAssignmentLineages"] = serde_json::to_value(manual).map_err(err)?;
                let mut identifications = serde_json::Map::new();
                for u in &t.utterances {
                    if let Some(info) = &u.speaker_identification {
                        if u.speaker_ids.len() != 1 {
                            return Err("発言の識別情報が不正です".into());
                        }
                        identifications.insert(get("utterance_lineage",&u.id)?,serde_json::json!({"speakerLineage":get("speaker_lineage",&u.speaker_ids[0])?,"trackId":track,"startUs":u.start_us,"endUs":u.end_us,"similarity":info.similarity_milli as f64/1000.,"runnerUp":info.runner_up_milli.map(|n|n as f64/1000.),"needsReview":info.needs_review,"reason":info.reason}));
                    }
                }
                config["speakerIdentification"] = serde_json::Value::Object(identifications);
                tx.execute("INSERT INTO analysis_revisions(id,meeting_id,revision_no,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) VALUES(?1,?2,1,'manual_snapshot','building',?3,'{}',?4,?5,1,?6)",params![analysis,mid,m.manifest.hash,config.to_string(),"0".repeat(64),m.meeting.created_at]).map_err(err)?;
                let speakers: BTreeSet<_> = t.speakers.iter().map(|s| s.id.as_str()).collect();
                for s in &t.speakers {
                    tx.execute("INSERT INTO speakers(id,analysis_revision_id,lineage_id,label,display_name,input_scope,identification_state,created_at) VALUES(?1,?2,?3,?4,?5,'all','unknown',?6)",params![get("speakers",&s.id)?,analysis,get("speaker_lineage",&s.id)?,s.label,s.display_name,m.meeting.created_at]).map_err(err)?;
                }
                for s in &m.segments {
                    if s.speaker.as_deref().is_some_and(|s| !speakers.contains(s)) {
                        return Err("音声区間の話者参照が不正です".into());
                    }
                    tx.execute("INSERT INTO speaker_segments(id,meeting_id,analysis_revision_id,track_id,speaker_id,start_us,end_us,overlap_group_id,confidence) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![get("speaker_segments",&s.id)?,mid,analysis,track,s.speaker.as_deref().map(|s|get("speakers",s)).transpose()?,s.start,s.end,s.overlap.as_deref().map(|g|get("overlap_group",g)).transpose()?,s.confidence]).map_err(err)?;
                }
                for (position, u) in t.utterances.iter().enumerate() {
                    let uid = get("utterances", &u.id)?;
                    if u.speaker_ids.iter().any(|s| !speakers.contains(s.as_str())) {
                        return Err("発言の話者参照が不正です".into());
                    }
                    tx.execute("INSERT INTO utterances(id,meeting_id,analysis_revision_id,track_id,lineage_id,sequence,start_us,end_us,recognized_text,effective_text,attribution_state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![uid,mid,analysis,track,get("utterance_lineage",&u.id)?,position as i64,u.start_us,u.end_us,u.recognized_text,u.effective_text,u.attribution_state]).map_err(err)?;
                    if u.speaker_ids.iter().collect::<BTreeSet<_>>().len() != u.speaker_ids.len() {
                        return Err("発言の話者が重複しています".into());
                    }
                    for (i, s) in u.speaker_ids.iter().enumerate() {
                        tx.execute("INSERT INTO utterance_speakers(utterance_id,speaker_id,analysis_revision_id,role,assignment_source) VALUES(?1,?2,?3,?4,?5)",params![uid,get("speakers",s)?,analysis,if i==0{"primary"}else{"overlap"},if m.manual_assignment_lineages.binary_search(&u.id).is_ok(){"manual"}else{"inferred"}]).map_err(err)?;
                    }
                }
                let hash = super::super::revisions::analysis_content_hash(&tx, &analysis)?;
                tx.execute("UPDATE analysis_revisions SET content_hash=?1,publication_state='committed' WHERE id=?2",params![hash,analysis]).map_err(err)?;
                tx.execute(
                    "UPDATE meetings SET current_analysis_id=?1 WHERE id=?2",
                    params![analysis, mid],
                )
                .map_err(err)?;
                for v in &m.matches {
                    if !speakers.contains(v.speaker.as_str()) {
                        return Err("人物対応の話者参照が不正です".into());
                    }
                    tx.execute("INSERT INTO voice_matches(id,meeting_id,analysis_revision_id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![get("voice_matches",&v.id)?,mid,analysis,get("speakers",&v.speaker)?,v.person.as_deref().map(|p|identity("voice_people","",p)).transpose()?,v.name,v.state,v.similarity,v.model,v.policy,v.created_at]).map_err(err)?;
                }
            } else if !m.matches.is_empty()
                || !m.segments.is_empty()
                || !m.manual_assignment_lineages.is_empty()
            {
                return Err("人物対応・音声区間の文字起こしがありません".into());
            }
            write_book(&tx, Some((old, &mid)), &m.vocabulary, identity)?;
            if let Some(markdown) = &m.markdown {
                super::super::minutes_markdown::validate_text(markdown)?;
                tx.execute("INSERT INTO minutes_markdown(meeting_id,markdown,content_hash,lock_version,operation_id,base_lock_version,updated_at) VALUES(?1,?2,?3,1,?4,0,?5)",params![mid,markdown,super::super::minutes_markdown::content_hash(markdown),operation,m.meeting.updated_at]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        crate::operation_observation::report("building_meetings", total, total, "meetings");
        super::super::voice::validate_registry(&db)?;
        portable::validate_scopes(&db)?;
        validate_database(&mut db, cancelled)?;
        Ok(db)
    }
}

fn insert_asset(
    db: &Connection,
    meeting: &str,
    id: &str,
    path: &str,
    media: &str,
    a: &Asset,
    missing: bool,
) -> Result<(), String> {
    if a.size < 0 || !portable::valid_hash(&a.hash) {
        return Err("音声ファイルのサイズ・ハッシュが不正です".into());
    }
    db.execute("INSERT INTO file_assets(id,meeting_id,storage_scope,relative_path,media_type,size_bytes,hash_algorithm,content_hash,state,created_at,updated_at) VALUES(?1,?2,'meeting',?3,?4,?5,'sha256',?6,?7,0,0)",params![id,meeting,path,media,a.size,a.hash,if missing{"missing"}else{"ready"}]).map_err(err)?;
    Ok(())
}
fn write_book(
    db: &Connection,
    meeting: Option<(&str, &str)>,
    entries: &[VocabularyEntry],
    identity: &impl Fn(&str, &str, &str) -> Result<String, String>,
) -> Result<(), String> {
    super::super::vocabulary::validate(entries)?;
    let scope = meeting.map_or_else(|| "global".into(), |(_, new)| format!("meeting:{new}"));
    db.execute("INSERT INTO vocabulary_scopes(id,meeting_id,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,0)",params![scope,meeting.map(|(_,new)|new),"0".repeat(64)]).map_err(err)?;
    for (position, e) in entries.iter().enumerate() {
        db.execute("INSERT INTO vocabulary_entries(id,scope_id,position,term,reading,note,enabled,lock_version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,1,0,0)",params![identity("vocabulary_entries",meeting.map_or("",|(old,_)|old),&e.id)?,scope,position as i64,e.term,e.reading,e.note,e.enabled]).map_err(err)?;
    }
    let hash = super::super::vocabulary::stored_book_hash(db, &scope)?;
    db.execute(
        "UPDATE vocabulary_scopes SET content_hash=?1 WHERE id=?2",
        params![hash, scope],
    )
    .map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn meeting_wav_verification_accepts_riff_and_long_rf64_without_allocating_pcm() {
        for frames in [160, i64::from(u32::MAX) / 2] {
            let header = crate::audio::wave::header(frames as u64).unwrap();
            let size = frames as u64 * 2 + header.len() as u64;
            let mut file = Cursor::new(header.clone());
            verify_meeting_wav(&mut file, size, frames).unwrap();
            assert!(verify_meeting_wav(&mut file, size, frames - 1).is_err());
            assert!(verify_meeting_wav(&mut file, size - 2, frames).is_err());
            // Corrupt the PCM channel count in either header layout.
            let mut stereo = header;
            let channels = stereo.len() - 22;
            stereo[channels..channels + 2].copy_from_slice(&2u16.to_le_bytes());
            assert!(verify_meeting_wav(&mut Cursor::new(stereo), size, frames).is_err());
        }
    }

    #[test]
    fn rf64_backup_audio_keeps_integrity_checks_and_strict_voice_clip_format() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().into()).unwrap();
        archive::tests::meeting(&storage, "rf64-audio");
        std::fs::create_dir(stage.path().join("assets")).unwrap();
        let (mut contents, _) = Contents::capture(
            &mut storage.connection.lock().unwrap(),
            stage.path(),
            &deleted_audio::Native::empty(),
            &|| false,
        )
        .unwrap();
        // A short RF64 fixture exercises the full hash/format path without
        // creating or hashing several GiB of PCM. The long header is tested above.
        let frames = 160u64;
        let size = frames * 2 + 80;
        let mut bytes = crate::audio::wave::header(u64::from(u32::MAX) / 2).unwrap();
        bytes[20..28].copy_from_slice(&(size - 8).to_le_bytes());
        bytes[28..36].copy_from_slice(&(frames * 2).to_le_bytes());
        bytes[36..44].copy_from_slice(&frames.to_le_bytes());
        bytes.resize(size as usize, 0);
        let path = stage.path().join("meeting.rf64.wav");
        std::fs::write(&path, &bytes).unwrap();
        let audio = contents.meetings[0].audio.as_mut().unwrap();
        audio.frames = frames as i64;
        audio.asset.size = size as i64;
        audio.asset.hash = format!("{:x}", Sha256::digest(&bytes));
        let asset = audio.asset.clone();
        let assets = BTreeMap::from([(asset.id.clone(), path.clone())]);
        contents.verify_audio(&assets, &|| false).unwrap();
        assert!(contents.verify_audio(&assets, &|| true).is_err());
        bytes[size as usize - 1] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(contents.verify_audio(&assets, &|| false).is_err());
        bytes[size as usize - 1] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        contents.people.push(Person {
            id: "person".into(),
            name: "登録者".into(),
            created_at: 0,
            samples: vec![Sample {
                id: asset.id.clone(),
                model: "a".repeat(64),
                extractor: crate::voice::EXTRACTOR.into(),
                dimension: 2,
                vector: "".into(),
                source_meeting: None,
                source_analysis: None,
                source_track: None,
                start: 0,
                end: 10_000,
                created_at: 0,
                audio: Some(asset),
                frames: Some(frames as i64),
            }],
        });
        // Registered clips retain their existing 44-byte RIFF requirement.
        assert!(contents.verify_audio(&assets, &|| false).is_err());
    }
}
