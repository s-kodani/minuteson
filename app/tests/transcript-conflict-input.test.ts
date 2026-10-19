import { test } from "node:test";
import assert from "node:assert/strict";
import { planConflictInput, resolveConflictInput, validateConflictPayload } from "../src/transcript-conflict-input.ts";
import { splitUtterance, type Analysis, type DraftPayload } from "../src/transcript-state.ts";
import type { TextEditSession } from "../src/transcript-edit-session.ts";
import type { SpeakerNameSession } from "../src/speaker-name-edit.ts";
import type { TranscriptRecovery } from "../src/transcript-recovery-save.ts";
const base: Analysis = { id: "a",meetingId: "m",contentHash: "a".repeat(64),speakers: [{ id: "s",label: "話者A",displayName: "元名" },{ id: "t",label: "話者B",displayName: null }],
  utterances: [{ id: "u",startUs: 0,endUs: 10_000_000,recognizedText: "原文です",effectiveText: "原文です",attributionState: "assigned",speakerIds: ["s"] }] };
const empty = (): DraftPayload => ({ schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},speaker_names: [],utterances: [] });
const text = (value = "手元入力"): TextEditSession => ({ utteranceId: "u",text: "旧開始時本文",speakerIds: ["s"],hadOverride: false,value });
const name = (value = "手元名"): SpeakerNameSession => ({ speakerId: "s",name: "旧名",hadOverride: false,value });
const record = (texts = [text("保存済み入力")],names = [name("保存済み名")]) => ({ payload: { schemaVersion: 4,basePayload: empty(),buffers: texts,nameBuffers: names,discardedFrom: null } } as TranscriptRecovery<DraftPayload>);
test("sparse body comparison keeps independent local text and latest assignment without false conflicts", () => {
  const local = empty(), latest = empty();
  local.utterances.push({ utterance_id: "u",effective_text: "手元変更",speaker_ids: ["s"] });
  latest.utterances.push({ utterance_id: "u",effective_text: "原文です",speaker_ids: ["t"] });
  const plan = planConflictInput(base,empty(),local,latest,[],[],null), resolved = resolveConflictInput(base,plan,{},{});
  assert.equal(plan.body.rows.length,2); assert.ok(plan.body.rows.every(r => !r.conflict));
  assert.deepEqual(resolved.payload.utterances,[{ utterance_id: "u",effective_text: "手元変更",speaker_ids: ["t"] }]);
});
test("removed text override uses original text while explicit null name remains distinct from absence", () => {
  const before = empty(), latest = empty(); before.utterances = [{ utterance_id: "u",effective_text: "旧変更",speaker_ids: ["s"] }];
  before.speaker_names = [{ speaker_id: "t",display_name: "旧名" }];
  latest.utterances = [{ utterance_id: "u",effective_text: "最新変更",speaker_ids: ["s"] }]; latest.speaker_names = [{ speaker_id: "t",display_name: null }];
  const plan = planConflictInput(base,before,empty(),latest,[],[],null);
  assert.equal(plan.body.rows.find(r => r.kind === "text")?.local,"原文です"); assert.equal(plan.body.rows.find(r => r.kind === "name")?.local,undefined);
  assert.equal(plan.body.rows.find(r => r.kind === "name")?.latest,null);
  assert.throws(() => resolveConflictInput(base,plan,{},{}),/項目ごと/);
  assert.deepEqual(resolveConflictInput(base,plan,Object.fromEntries(plan.body.rows.map(r => [r.key,"local"])),{}).payload,empty());
  assert.deepEqual(resolveConflictInput(base,plan,Object.fromEntries(plan.body.rows.map(r => [r.key,"latest"])),{}).payload.speaker_names,[{ speaker_id: "t",display_name: null }]);
});
test("every local and stored mixed input needs an explicit choice and selected values remain unconfirmed", () => {
  const latest = { ...empty(),speaker_names: [{ speaker_id: "s",display_name: "最新名" }] };
  const plan = planConflictInput(base,empty(),empty(),latest,[text()],[name()],record());
  assert.throws(() => resolveConflictInput(base,plan,{},{}),/未確定入力ごと/);
  const selected = resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,r.kind === "text" ? "local" : "stored"])));
  assert.deepEqual(selected.payload,latest); assert.equal(selected.texts[0].value,"手元入力"); assert.equal(selected.texts[0].text,"原文です");
  assert.equal(selected.names[0].value,"保存済み名"); assert.equal(selected.names[0].name,"最新名"); assert.equal(selected.names[0].hadOverride,true);
});
test("stored-only input is included and explicit omission never modifies the saved record", () => {
  const saved = record(), before = structuredClone(saved), plan = planConflictInput(base,empty(),empty(),empty(),[],[],saved);
  assert.equal(plan.inputs.length,2); assert.ok(plan.inputs.every(r => r.local === null));
  assert.throws(() => resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,"local"]))),/側に入力/);
  assert.deepEqual(resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,"omit"]))).texts,[]);
  assert.deepEqual(saved,before);
});
test("a hidden merged name cannot partially resume other selected text", () => {
  const latest = { ...empty(),schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},speaker_merges: [{ source_id: "s",target_id: "t" }] };
  const plan = planConflictInput(base,empty(),empty(),latest,[text()],[name()],null);
  assert.throws(() => resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,"local"]))),/選択した話者/);
  const selected = resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,r.kind === "name" ? "omit" : "local"])));
  assert.equal(selected.texts.length,1); assert.deepEqual(selected.texts[0].speakerIds,["t"]);
});
test("a split-away original buffer is copyable but cannot be invisibly resumed", () => {
  const latest = splitUtterance(base,empty(),"u",5_000_000,2,["split:00000000-0000-4000-8000-000000000001","split:00000000-0000-4000-8000-000000000002"]);
  const plan = planConflictInput(base,empty(),empty(),latest,[text()],[],null);
  assert.throws(() => resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,"local"]))),/選択した発言/);
  assert.equal(resolveConflictInput(base,plan,{},Object.fromEntries(plan.inputs.map(r => [r.key,"omit"]))).payload.utterance_splits?.length,1);
});
test("incompatible speaker dependency choices are rejected before returning resumed state", () => {
  const id = "manual:00000000-0000-4000-8000-000000000003", local = { ...empty(),schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},manual_speakers: [{ id,label: "追加" }],speaker_names: [{ speaker_id: id,display_name: "手元" }] };
  const latest = empty(), plan = planConflictInput(base,empty(),local,latest,[],[],null);
  const choices = Object.fromEntries(plan.body.rows.filter(r => r.kind === "manual").map(r => [r.key,"latest" as const]));
  assert.throws(() => resolveConflictInput(base,plan,choices,{}),/話者名の参照/);
  assert.equal(resolveConflictInput(base,plan,{},{}).payload.manual_speakers?.length,1);
});
test("split source text, contiguous times, ID uniqueness and speaker assignments are preflighted", () => {
  const good = splitUtterance(base,empty(),"u",5_000_000,2,["split:00000000-0000-4000-8000-000000000001","split:00000000-0000-4000-8000-000000000002"]);
  validateConflictPayload(base,good);
  for(const change of ["text","gap","duplicate","speaker","empty","end"]) {
    const bad = structuredClone(good), split = bad.utterance_splits![0];
    if(change === "text") split.source_effective_text = "不一致";
    if(change === "gap") split.parts[1].start_us++;
    if(change === "duplicate") split.parts[1].id = split.parts[0].id;
    if(change === "speaker") split.parts[0].speaker_ids = ["missing"];
    if(change === "empty") split.parts[0].effective_text = "";
    if(change === "end") split.parts[1].end_us--;
    assert.throws(() => validateConflictPayload(base,bad),undefined,change);
  }
});
test("manual IDs, cyclic merges, reference limits and payload schema are preflighted", () => {
  for(const patch of [{ schema_version: 99 },{ manual_speakers: [{ id: "manual:bad",label: "名" }],schema_version:4 },
    { schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},speaker_merges: [{ source_id: "s",target_id: "t" },{ source_id: "t",target_id: "s" }] },
    { speaker_names: [{ speaker_id: "missing",display_name: "名" }] },{ speaker_names: [{ speaker_id: "s",display_name: "あ".repeat(201) }] },
    { utterances: [{ utterance_id: "missing",effective_text: "文",speaker_ids: [] }] },
    { utterances: [{ utterance_id: "u",effective_text: "文",speaker_ids: ["s","s"] }] }]) assert.throws(() => validateConflictPayload(base,{ ...empty(),...patch }));
});
test("unknown and duplicate input choices cannot discard other input, and snapshots are immutable", () => {
  const raw = text(), saved = record(), plan = planConflictInput(base,empty(),empty(),empty(),[raw],[],saved);
  raw.value = "外部変更"; saved.payload.buffers[0].value = "外部変更";
  assert.equal(plan.inputs[0].local?.value,"手元入力"); assert.equal(plan.inputs[0].stored?.value,"保存済み入力");
  assert.throws(() => { plan.inputs[0].local!.value = "改変"; },TypeError);
  assert.throws(() => resolveConflictInput(base,plan,{}, { foreign: "omit" }),/比較していない/);
  assert.throws(() => planConflictInput(base,empty(),empty(),empty(),[text(),text()],[],null),/重複/);
  const bad = empty(); bad.utterances = [{ utterance_id: "u",effective_text: "文",speaker_ids: [] },{ utterance_id: "u",effective_text: "文",speaker_ids: [] }];
  assert.throws(() => planConflictInput(base,empty(),bad,empty(),[],[],null),/重複/);
});
