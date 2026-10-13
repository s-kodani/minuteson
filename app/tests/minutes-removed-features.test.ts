import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { installPlainMinutesWindowFixture } from "./plain-minutes-window-fixture.js";

const obsoleteCommands = ["start_minutes_generation","get_minutes_exports","export_minutes_markdown","get_minutes_export_status","get_minutes_versions","get_minutes_preview","get_minutes_draft","create_minutes_draft","save_minutes_draft","undo_minutes_edit","adopt_minutes_candidate","restore_minutes_version","get_minutes_evidence_source","get_minutes_transcript_page","prepare_minutes_transcript_playback","prepare_minutes_evidence_playback"];
test("obsolete minutes operations have no native handler, registration or client dispatch", () => {
  const native=readFileSync(new URL("../src-tauri/src/lib.rs",import.meta.url),"utf8");
  const player=readFileSync(new URL("../src/transcript-audio.tsx",import.meta.url),"utf8");
  for(const command of [...obsoleteCommands,"retry_minutes_generation","retry_transcription","get_capture_test_recovery","retry_capture_test_recovery"]) {
    assert.doesNotMatch(native,new RegExp("\\b"+command+"\\b"),command);
    assert.ok(!player.includes('"'+command+'"'),command);
  }
  for(const command of ["get_minutes_markdown","save_minutes_markdown","start_markdown_minutes_generation",
    "export_plain_minutes_markdown","get_plain_minutes_export_status","prepare_markdown_interval_playback"]) {
    assert.match(native,new RegExp("async fn "+command+"\\("),command);
    assert.match(native,new RegExp("^\\s+"+command+",$","m"),command);
  }
});
test("rich editor and evidence source implementations are removed, not merely hidden",()=>{
  for(const path of ["minutes-document.tsx","minutes-editor.tsx","minutes-evidence.tsx","minutes-pages.tsx","minutes-pagination.ts","minutes-rich-field.tsx","minutes-edit.ts","minutes-save.ts","fixed-transcript.tsx"]) {
    assert.equal(existsSync(new URL("../src/"+path,import.meta.url)),false,path);
  }
  assert.equal(existsSync(new URL("../src-tauri/src/storage/minutes_evidence.rs",import.meta.url)),false);
  const types=readFileSync(new URL("../src/minutes-state.ts",import.meta.url),"utf8");
  assert.doesNotMatch(types,/MinutesContent|Evidence|MinutesDraft|MinutesVersions|minutesExportRequest/);
  for(const path of ["style.css","prototype-alignment.css"]) {
    const css=readFileSync(new URL("../src/"+path,import.meta.url),"utf8");
    assert.doesNotMatch(css,/\.(?:minutes-(?:document|editor|rich-field|split|toolbar|reference)|fixed-(?:evidence|transcript)|evidence-)[\w-]*/);
  }
});
test("plain-window fixture refuses obsolete operations instead of implementing removed features",async()=>{
  const previous=globalThis.window;
  globalThis.window={};
  try {
    await installPlainMinutesWindowFixture({addInitScript:async initialize=>initialize(),reload:async()=>{}});
    const invoke=globalThis.window.__TAURI_INTERNALS__.invoke;
    assert.equal((await invoke("get_current_analysis",{meetingId:"m1"})).id,"analysis");
    for(const command of obsoleteCommands)await assert.rejects(invoke(command),/unmocked command/);
    assert.equal(globalThis.window.__mon.views,undefined);
    assert.equal(globalThis.window.__mon.drafts,undefined);
  }finally{globalThis.window=previous;}
});
test("legacy draft edits, adoption and version restoration are deleted from native storage",()=>{
  const root=new URL("../src-tauri/src/",import.meta.url);
  assert.equal(existsSync(new URL("storage/minutes_ui.rs",root)),false);
  assert.equal(existsSync(new URL("storage/minutes_ui_tests.rs",root)),false);
  assert.equal(existsSync(new URL("storage/minutes_drafts.rs",root)),false);
  const revisions=readFileSync(new URL("storage/revisions.rs",root),"utf8");
  assert.doesNotMatch(revisions,/fn (?:commit_minutes_candidate|commit_minutes_inner|restore_previous_minutes|restore_minutes_inner)\b|struct CommittedMinutes\b/);
  // ASR revision publication is a separate, still required operation.
  assert.match(revisions,/fn commit_analysis_inner\(/);
  const modules=readFileSync(new URL("storage.rs",root),"utf8");
  assert.match(modules,/#\[cfg\(test\)\]\npub\(crate\) mod current_minutes_fixture;/);
  assert.doesNotMatch(modules,/mod minutes_ui;|MinutesAdoptRequest|MinutesVersionState|CommittedMinutes/);
  assert.equal(existsSync(new URL("storage/stored_minutes_fixture.rs",root)),false);
});
test("structured minutes export is removed and only exact Markdown publishing remains",()=>{
  const root=new URL("../src-tauri/src/",import.meta.url);
  const shared=readFileSync(new URL("storage/exports.rs",root),"utf8");
  assert.doesNotMatch(shared,/\b(?:ExportRequest|PreparedExport|MinutesContent|MinutesInput)\b|fn (?:prepare_export|write_export|write_export_using|existing_export|checked_source)\b/);
  assert.match(shared,/pub\(super\) fn (?:record|safe_filename)\(/);
  const modules=readFileSync(new URL("storage.rs",root),"utf8");
  assert.doesNotMatch(modules,/\bExportRequest\b/);
  const writer=readFileSync(new URL("storage/markdown_exports.rs",root),"utf8");
  for(const name of ["prepare_markdown_export","write_markdown_export","existing_markdown_export"])
    assert.match(writer,new RegExp("fn "+name+"\\("));
  assert.doesNotMatch(writer,/minutes_drafts|minutes::|\.markdown\(/);
  assert.match(writer,/prepared\.document\.markdown\.as_bytes\(\)/);
});
test("structured worker execution and candidate publication are removed from the product",()=>{
  const root=new URL("../src-tauri/src/",import.meta.url);
  const worker=readFileSync(new URL("generation.rs",root),"utf8");
  assert.doesNotMatch(worker,/fn (?:run_task|run_merge|save_items|save_task_output|enqueue_version|enqueue_previous|enqueue_legacy|enqueue_prompt_v3)\b|llama::run_unit|finish_minutes_candidate/);
  assert.match(worker,/if !markdown_job::is_markdown\(&spec\)/);
  assert.match(worker,/markdown_job::process\(storage, job_id, stopping, progress\)/);
  assert.equal(existsSync(new URL("generation/stored_fixture.rs",root)),false);
  assert.doesNotMatch(worker,/mod adaptive;|mod merge;|mod plan;|mod result;|struct Parameters|struct Checkpoint/);
  for(const path of ["storage/minutes.rs","storage/minutes_drafts.rs","minutes_document.rs","storage/stored_minutes_fixture.rs"]) assert.equal(existsSync(new URL(path,root)),false,path);
  const admission=readFileSync(new URL("storage/minutes_input.rs",root),"utf8");
  for(const name of ["enqueue_minutes_job","enqueue_minutes_job_configured"])
    assert.match(admission,new RegExp("#\\[cfg\\(test\\)\\]\\n    pub\\(crate\\) fn "+name+"\\("));
});
test("opening plain minutes never migrates or publishes a removed structured document",()=>{
  const root=new URL("../src-tauri/src/",import.meta.url);
  const storage=readFileSync(new URL("storage/minutes_markdown.rs",root),"utf8");
  assert.doesNotMatch(storage,/import_current_document|require_read_current|plain_markdown|minutes_drafts::|minutes::read_content|current_minutes_id/);
  const getter=storage.slice(storage.indexOf("pub(crate) fn get_minutes_markdown("),storage.indexOf("pub(crate) fn save_minutes_markdown("));
  assert.match(getter,/read\(&db, meeting\)/);
  assert.doesNotMatch(getter,/transaction|require_data_writable|\.execute|save\(/);
  assert.equal(existsSync(new URL("minutes_document.rs",root)),false);
});
