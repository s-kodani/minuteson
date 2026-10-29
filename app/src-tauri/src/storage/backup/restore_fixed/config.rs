use super::*;

impl FixedData<'_, '_> {
    pub(in crate::storage::backup) fn audio_manifest_hash(
        &self,
        meeting: &str,
        old: &str,
    ) -> Result<String, String> {
        let mut query = self
            .registry
            .prepare("SELECT DISTINCT new_hash FROM assets WHERE meeting=?1 AND old_hash=?2")
            .map_err(|e| e.to_string())?;
        let hashes = query
            .query_map(params![meeting, old], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if hashes.len() != 1 {
            return Err("音声manifestの由来が欠損・曖昧か再構成前です".into());
        }
        Ok(hashes[0].clone())
    }

    pub(in crate::storage::backup) fn parameters(
        &self,
        meeting: &str,
        input_analysis: Option<&str>,
        original: &Value,
    ) -> Result<Value, String> {
        crate::job_codec::parameters(meeting, input_analysis, original, self)
    }

    fn manual_lineages(
        &self,
        meeting: &str,
        input: &str,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        self.ids.get("analysis_revisions", meeting, input)?;
        let mut query = self.ids.source().database.prepare(
            "SELECT u.lineage_id FROM diarization_manual_assignments d JOIN utterances u ON u.id=d.utterance_id AND u.analysis_revision_id=d.analysis_id JOIN analysis_revisions a ON a.id=d.analysis_id WHERE a.id=?1 AND a.meeting_id=?2 ORDER BY u.lineage_id"
        ).map_err(|e|e.to_string())?;
        let rows = query
            .query_map(params![input, meeting], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub(in crate::storage::backup) fn job_spec(
        &self,
        meeting: &str,
        id: &str,
    ) -> Result<JobSpec, String> {
        let original = self.source_job_spec(meeting, id)?;
        let input = original.input_analysis_id;
        let hash = original.input_hash;
        let input_hash = if let Some(input) = &input {
            self.resolve("analysis_revisions", meeting, input, &hash)?
        } else {
            self.audio_manifest_hash(meeting, &hash)?
        };
        Ok(JobSpec {
            meeting_id: self.ids.get("meetings", "", meeting)?,
            kind: original.kind,
            operation_id: self.ids.get("operation", meeting, &original.operation_id)?,
            input_analysis_id: input
                .as_deref()
                .map(|id| self.ids.get("analysis_revisions", meeting, id))
                .transpose()?,
            input_hash,
            profile_snapshot: original.profile_snapshot,
            parameters: self.parameters(meeting, input.as_deref(), &original.parameters)?,
            total_units: original.total_units,
        })
    }

    pub(in crate::storage::backup) fn source_job_spec(
        &self,
        meeting: &str,
        id: &str,
    ) -> Result<JobSpec, String> {
        self.ids.get("processing_jobs", meeting, id)?;
        let (kind,operation,input,hash,profile,parameters,total):(String,String,Option<String>,String,String,String,Option<i64>)=self.ids.source().database.query_row("SELECT kind,operation_id,input_analysis_id,input_hash,profile_snapshot_json,parameters_json,total_units FROM processing_jobs WHERE id=?1 AND meeting_id=?2",params![id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|e|e.to_string())?;
        Ok(JobSpec {
            meeting_id: meeting.into(),
            kind: serde_json::from_value(json!(kind)).map_err(|e| e.to_string())?,
            operation_id: operation,
            input_analysis_id: input,
            input_hash: hash,
            profile_snapshot: serde_json::from_str(&profile).map_err(|e| e.to_string())?,
            parameters: serde_json::from_str(&parameters).map_err(|e| e.to_string())?,
            total_units: total,
        })
    }

    /// The target already contains mapped relational rows. No live DB is changed.
    pub(in crate::storage::backup) fn analysis_config(
        &self,
        meeting: &str,
        id: &str,
        target: &Connection,
    ) -> Result<(Value, Option<InputSnapshot>), String> {
        let text: String = self
            .ids
            .source()
            .database
            .query_row(
                "SELECT config_json FROM analysis_revisions WHERE id=?1 AND meeting_id=?2",
                params![id, meeting],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let original: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut value = original.clone();
        if !value.is_object() {
            return Err("解析条件はオブジェクトである必要があります".into());
        }
        if let Some(job) = original.get("jobId").and_then(Value::as_str) {
            let hash = original
                .get("specHash")
                .and_then(Value::as_str)
                .ok_or("解析ジョブの固定ハッシュがありません")?;
            value["specHash"] = json!(self.resolve("processing_jobs", meeting, job, hash)?);
            value["jobId"] = json!(self.ids.get("processing_jobs", meeting, job)?);
        }
        if let Some(job) = original.get("retryOfJobId").and_then(Value::as_str) {
            value["retryOfJobId"] = json!(self.ids.get("processing_jobs", meeting, job)?);
        }
        if let Some(input) = original.get("inputAnalysisId").and_then(Value::as_str) {
            let hash = original
                .get("inputContentHash")
                .and_then(Value::as_str)
                .ok_or("固定入力のハッシュがありません")?;
            value["inputContentHash"] =
                json!(self.resolve("analysis_revisions", meeting, input, hash)?);
            value["inputAnalysisId"] = json!(self.ids.get("analysis_revisions", meeting, input)?);
        }
        if let Some(vocabulary) = original.get("vocabulary").filter(|v| !v.is_null()) {
            value["vocabulary"] = self.vocabulary(meeting, vocabulary)?;
        }
        if let Some(tracks) = original.get("tracks") {
            let mut tracks: Vec<crate::diarization::TrackSettings> =
                serde_json::from_value(tracks.clone()).map_err(|e| e.to_string())?;
            for track in &mut tracks {
                track.track_id = self.ids.get("audio_tracks", meeting, &track.track_id)?;
            }
            value["tracks"] = serde_json::to_value(tracks).map_err(|e| e.to_string())?;
        }
        if let Some(entries) = original
            .get("speakerIdentification")
            .and_then(Value::as_object)
        {
            let mut mapped = serde_json::Map::new();
            for (lineage, info) in entries {
                let mut info = info.clone();
                if let Some(owner) = info["speakerLineage"].as_str() {
                    info["speakerLineage"] =
                        json!(self.ids.get("speaker_lineage", meeting, owner)?);
                }
                if let Some(track) = info["trackId"].as_str() {
                    info["trackId"] = json!(self.ids.get("audio_tracks", meeting, track)?);
                }
                mapped.insert(self.ids.get("utterance_lineage", meeting, lineage)?, info);
            }
            value["speakerIdentification"] = Value::Object(mapped);
        }
        if let Some(completion) = original.get("speakerCompletion") {
            let job = completion["jobId"]
                .as_str()
                .ok_or("話者補完の元ジョブがありません")?;
            value["speakerCompletion"]["jobId"] =
                json!(self.ids.get("processing_jobs", meeting, job)?);
        }
        if let Some(update) = original.get("speakerAssignmentUpdate") {
            if update["schemaVersion"] != 1 || update["policy"] != "all_detected_v1" {
                return Err("話者割り当て更新の履歴形式が不正です".into());
            }
            let draft: Option<crate::storage::DraftStamp> =
                serde_json::from_value(update["sourceDraft"].clone()).map_err(|e| e.to_string())?;
            if let Some(mut draft) = draft {
                draft.id = self.ids.get("working_drafts", meeting, &draft.id)?;
                value["speakerAssignmentUpdate"]["sourceDraft"] =
                    serde_json::to_value(draft).map_err(|e| e.to_string())?;
            }
        }
        if let Some(consolidation) = original.get("utteranceConsolidation") {
            let mut provenance: crate::storage::transcript_consolidation::Provenance =
                serde_json::from_value(consolidation.clone()).map_err(|e| e.to_string())?;
            if provenance.schema_version != 1 {
                return Err("発言統合の履歴形式が不正です".into());
            }
            for group in &mut provenance.groups {
                group.lineage_id = self
                    .ids
                    .get("utterance_lineage", meeting, &group.lineage_id)?;
                for source in &mut group.source_lineages {
                    *source = self.ids.get("utterance_lineage", meeting, source)?;
                }
            }
            if let Some(draft) = &mut provenance.source_draft {
                draft.id = self.ids.get("working_drafts", meeting, &draft.id)?;
            }
            value["utteranceConsolidation"] =
                serde_json::to_value(provenance).map_err(|e| e.to_string())?;
        }
        if let Some(operations) = original.get("manualSpeakerOperations") {
            let mut operations: ManualSpeakerOperations =
                serde_json::from_value(operations.clone()).map_err(|e| e.to_string())?;
            for speaker in &mut operations.added {
                speaker.id = self.ids.get("speakers", meeting, &speaker.id)?;
            }
            for merge in &mut operations.merges {
                merge.source_id = self.ids.get("speakers", meeting, &merge.source_id)?;
                merge.target_id = self.ids.get("speakers", meeting, &merge.target_id)?;
            }
            value["manualSpeakerOperations"] =
                serde_json::to_value(operations).map_err(|e| e.to_string())?;
        }
        if let Some(operations) = original.get("manualUtteranceSplits") {
            let mut operations: Vec<SplitOperation> =
                serde_json::from_value(operations.clone()).map_err(|e| e.to_string())?;
            for operation in &mut operations {
                operation.source_content_hash = self.resolve(
                    "analysis_revisions",
                    meeting,
                    &operation.source_analysis_id,
                    &operation.source_content_hash,
                )?;
                operation.source_analysis_id =
                    self.ids
                        .get("analysis_revisions", meeting, &operation.source_analysis_id)?;
                let mut payload = TranscriptDraftPayload {
                    schema_version: crate::current_formats::TRANSCRIPT_DRAFT,
                    utterance_splits: operation.splits.clone(),
                    ..Default::default()
                };
                self.ids.transcript(meeting, &mut payload)?;
                operation.splits = payload.utterance_splits;
            }
            value["manualUtteranceSplits"] =
                serde_json::to_value(operations).map_err(|e| e.to_string())?;
        }
        if let Some(ranges) = value.get_mut("reviewRanges").and_then(Value::as_array_mut) {
            for range in ranges {
                let track = range["trackId"]
                    .as_str()
                    .ok_or("要確認範囲のトラックがありません")?
                    .to_owned();
                range["trackId"] = json!(self.ids.get("audio_tracks", meeting, &track)?);
            }
        }
        if let Some(lineages) = original.get("sourceRecognitionLineages") {
            let old: Vec<String> =
                serde_json::from_value(lineages.clone()).map_err(|e| e.to_string())?;
            value["sourceRecognitionLineages"] = json!(old
                .iter()
                .map(|id| self.ids.get("utterance_lineage", meeting, id))
                .collect::<Result<Vec<_>, _>>()?);
        }
        let input = if let Some(snapshot) = original.get("inputSnapshot") {
            let mut snapshot: InputSnapshot =
                serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?;
            if snapshot.schema_version != 1 {
                return Err("固定入力の形式が不正です".into());
            }
            let (source,hash,draft,working,title,created,started):(String,String,Option<String>,String,String,i64,Option<i64>)=self.ids.source().database.query_row("SELECT source_analysis_id,source_content_hash,source_draft_json,working_content_hash,meeting_title,meeting_created_at,meeting_started_at_utc_ms FROM analysis_input_snapshots WHERE analysis_id=?1 AND meeting_id=?2",params![id,meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|e|e.to_string())?;
            if original["inputSnapshot"]
                != json!({"schemaVersion":1,"sourceAnalysisId":source,"sourceContentHash":hash,"sourceDraft":draft.as_ref().map(|d|serde_json::from_str::<Value>(d)).transpose().map_err(|e|e.to_string())?,"workingContentHash":working,"meetingTitle":title,"meetingCreatedAt":created,"meetingStartedAtUtcMs":started})
            {
                return Err("固定入力の由来が保存情報と一致しません".into());
            }
            snapshot.source_content_hash = self.resolve(
                "analysis_revisions",
                meeting,
                &snapshot.source_analysis_id,
                &snapshot.source_content_hash,
            )?;
            snapshot.source_analysis_id =
                self.ids
                    .get("analysis_revisions", meeting, &snapshot.source_analysis_id)?;
            if let Some(draft) = &mut snapshot.source_draft {
                draft.content_hash =
                    self.resolve("working_drafts", meeting, &draft.id, &draft.content_hash)?;
                draft.id = self.ids.get("working_drafts", meeting, &draft.id)?;
            }
            let view = super::super::super::analysis::read_stored_analysis_content(
                target,
                &self.ids.get("meetings", "", meeting)?,
                &self.ids.get("analysis_revisions", meeting, id)?,
                "",
            )?;
            snapshot.working_content_hash = super::super::super::minutes_input::working_hash(
                target,
                &view,
                &snapshot.meeting_title,
                snapshot.meeting_created_at,
                snapshot.meeting_started_at_utc_ms,
                None,
            )?;
            value["inputSnapshot"] = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
            Some(snapshot)
        } else {
            None
        };
        if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
            return Err("再構成した解析条件が容量上限を超えています".into());
        }
        Ok((value, input))
    }
}

impl crate::job_codec::Resolver for FixedData<'_, '_> {
    fn id(
        &self,
        domain: crate::job_codec::ReferenceDomain,
        meeting: &str,
        id: &str,
    ) -> Result<String, String> {
        self.ids.get(
            domain.name(),
            if domain.global() { "" } else { meeting },
            id,
        )
    }
    fn hash(
        &self,
        domain: crate::job_codec::ReferenceDomain,
        meeting: &str,
        id: &str,
        hash: &str,
    ) -> Result<String, String> {
        self.resolve(
            domain.name(),
            if domain.global() { "" } else { meeting },
            id,
            hash,
        )
    }
    fn audio_plan(
        &self,
        meeting: &str,
        plan: &RecordedAudioPlan,
        known: Option<&crate::storage::KnownInput>,
    ) -> Result<RecordedAudioPlan, String> {
        let value = self.analysis_audio_plan(
            meeting,
            &serde_json::to_value(plan).map_err(|e| e.to_string())?,
            known,
        )?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
    fn person_exists(&self, id: &str) -> Result<bool, String> {
        self.ids
            .source()
            .database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM voice_people WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
    fn manual_lineages(
        &self,
        meeting: &str,
        input: &str,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        FixedData::manual_lineages(self, meeting, input)
    }
    fn request(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        FixedData::request(self, meeting, value)
    }
    fn vocabulary(&self, meeting: &str, value: &Value) -> Result<Value, String> {
        FixedData::vocabulary(self, meeting, value)
    }
}
