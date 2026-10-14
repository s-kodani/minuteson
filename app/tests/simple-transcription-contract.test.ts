import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
const native = (name: string) => readFileSync(new URL(`../src-tauri/src/${name}`, import.meta.url), "utf8");

test("meeting UI has no candidates, model preferences or global editing options", () => {
  const view = source("analysis.tsx") + source("transcript.tsx");
  assert.doesNotMatch(view, /adopt_analysis|restore_analysis|save_analysis_preferences|save_diarization_preferences|undo_transcript_edit|splitUtterance|mergeSpeaker|TranscriptRecovery|DiarizationPanel|文字起こしの編集オプション|編集オプション/);
  assert.match(source("analysis.tsx"), /start_analysis_flow.*meetingId/);
  assert.match(source("analysis.tsx"), /await flushPendingEdits\("transcription"\)/);
  assert.doesNotMatch(source("analysis.tsx"),/retryOperations|retry_transcription|retry_diarization|>再試行<\/button>/);
});
test("removed editing and candidate commands are not exposed by the application", () => {
  const ipc = native("lib.rs");
  for (const name of ["adopt_analysis_candidate", "restore_analysis_version", "get_analysis_preview", "get_analysis_version_draft", "undo_transcript_edit", "save_transcript_edit_recovery", "start_transcription", "start_diarization", "get_speaker_reviews"]) assert.ok(!ipc.includes(name), name);
  for (const file of ["diarization-panel.tsx", "analysis-flow.tsx", "speaker-review.tsx", "transcript-recovery-panel.tsx", "transcript-conflict-panel.tsx", "version-publication.tsx"]) assert.equal(existsSync(new URL(`../src/${file}`, import.meta.url)), false, file);
  assert.doesNotMatch(native("storage.rs"), /mod speaker_review|use speaker_review/);
  assert.equal(existsSync(new URL("../src-tauri/src/storage/speaker_review.rs", import.meta.url)), false);
  for (const file of ["browser-fixture.js", "audio-deletion-browser-fixture.js"]) assert.doesNotMatch(readFileSync(new URL(file, import.meta.url), "utf8"), /get_speaker_reviews/);
});
test("individual editing retains IME guards, selection, identity and retry barriers", () => {
  const view = source("transcript.tsx");
  assert.match(view, /shouldOpenTranscriptBody\(event.detail, body.ownerDocument.getSelection/);
  assert.match(view, /event.nativeEvent.isComposing \|\| event.nativeEvent.keyCode === 229/);
  assert.match(view, /result.lockVersion !== expectedLockVersion \+ 1/);
  assert.match(view, /result.baseAnalysisId !== analysisRef.current\?\.id/);
  assert.match(view, /JSON.stringify\(canonicalPayload\(result.payload\)\) !== JSON.stringify\(canonicalPayload\(payload\)\)/);
  assert.match(view, /saved.baseAnalysisId !== base.id/);
  assert.match(view, /registerPendingEdits\(\(\) => barrier.current\(\)\)/);
  assert.match(view, /addingRef.current\) throw Error/);
});
test("individual speaker controls save only explicit input", () => {
  const view = source("transcript.tsx");
  assert.match(view, /className="speaker-name-button"[^\n]*openName\(speaker.id, event.currentTarget\)/);
  assert.match(view, /restoreTarget=\{\(\) => nameOpener.current\}/);
  assert.match(view, /className="utterance-edit-tools"/);
  assert.match(view, /assignSpeakers\(analysis, payload, \[utterance.id\]/);
  assert.match(view, /await ensureDraft\(\); queue.edit\(payload => addSpeaker/);
  assert.match(view, /await queue.retry\(\); if \(mounted.current\) setAdding\(false\)/);
});
test("imports no longer persist a meeting model profile", () => {
  const view = source("audio-import.tsx"), dto = native("audio_import.rs");
  assert.doesNotMatch(view, /import-analysis-profile|options:.*profile/);
  assert.doesNotMatch(view, /get_analysis_preferences|get_diarization_preferences|engineConfigured|diarizationConfigured|共通用語とAI設定を再確認/);
  assert.match(view, /全体設定で選んだモデルで文字起こしと話者分離を進めます/);
  assert.match(view, /モデルが未導入でも音声は保持します/);
  assert.doesNotMatch(dto.slice(dto.indexOf("pub struct ImportOptions"), dto.indexOf("impl ImportOptions")), /pub profile/);
  assert.doesNotMatch(native("storage/imports.rs"), /fn flow_profile/);
  assert.match(native("storage/analysis_flows.rs"), /unwrap_or\("standard"\)/);
});
test("automatic utterance publication checks ownership and commits head and stage in one transaction", () => {
  const storage = native("storage/analysis_flows.rs");
  assert.match(storage, /pub\(crate\) fn complete_analysis_flow/);
  assert.match(storage, /initial != format!\("flow:\{\}:\{kind\}", old.id\)/);
  assert.match(storage, /head.is_some\(\) \|\| super::drafts::load_draft/);
  assert.match(storage, /analysis_content_hash\(&tx, output\)\? != hash/);
  assert.match(storage, /publication_state='committed'/);
  assert.match(storage, /current_analysis_id=\?1,previous_analysis_id=NULL/);
  assert.match(storage, /tx.commit\(\)/);
  assert.match(native("analysis_flow.rs"), /storage.complete_analysis_flow\(flow\)/);
});
