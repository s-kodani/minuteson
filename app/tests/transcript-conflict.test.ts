import { test } from "node:test";
import assert from "node:assert/strict";
import { canonicalPayload, type DraftPayload } from "../src/transcript-state.ts";
import { planTranscriptConflict, resolveTranscriptConflict, type ConflictChoice } from "../src/transcript-conflict.ts";

const empty = (): DraftPayload => ({ schema_version:4,...{manual_speakers:[],speaker_merges:[],utterance_splits:[]},speaker_names: [],utterances: [] });
const payload = (): DraftPayload => ({ ...empty(),utterances: [{ utterance_id: "u",effective_text: "原文",speaker_ids: ["s"] }],speaker_names: [{ speaker_id: "s",display_name: "田中" }] });
const choose = (plan: ReturnType<typeof planTranscriptConflict>,kind: string,value: ConflictChoice) => Object.fromEntries(plan.rows.filter(row => row.kind === kind).map(row => [row.key,value]));
test("three-way comparison independently preserves local text and latest speaker assignment", () => {
  const base = payload(), local = payload(), latest = payload();
  local.utterances[0].effective_text = "手元の本文"; latest.utterances[0].speaker_ids = ["t"];
  const plan = planTranscriptConflict(base,local,latest), result = resolveTranscriptConflict(plan,{});
  assert.equal(plan.rows.length,2); assert.equal(plan.rows.some(row => row.conflict),false);
  assert.deepEqual(result.utterances,[{ utterance_id: "u",effective_text: "手元の本文",speaker_ids: ["t"] }]);
  assert.deepEqual(result.speaker_names,base.speaker_names);
});
test("same-field text and name conflicts require independent explicit choices", () => {
  const base = payload(), local = payload(), latest = payload();
  local.utterances[0].effective_text = "手元"; latest.utterances[0].effective_text = "最新";
  local.speaker_names[0].display_name = "手元名"; latest.speaker_names[0].display_name = "最新名";
  const plan = planTranscriptConflict(base,local,latest);
  assert.equal(plan.rows.filter(row => row.conflict).length,2);
  assert.throws(() => resolveTranscriptConflict(plan,{}),/項目ごと/);
  assert.throws(() => resolveTranscriptConflict(plan,choose(plan,"text","local")),/項目ごと/);
  const result = resolveTranscriptConflict(plan,{ ...choose(plan,"text","local"),...choose(plan,"name","latest") });
  assert.equal(result.utterances[0].effective_text,"手元"); assert.equal(result.speaker_names[0].display_name,"最新名");
});
test("identical concurrent edits are not conflicts and latest unrelated entries survive", () => {
  const base = payload(), local = payload(), latest = payload();
  local.speaker_names[0].display_name = latest.speaker_names[0].display_name = "一致した名";
  latest.speaker_names.push({ speaker_id: "t",display_name: "最新の別名" });
  const plan = planTranscriptConflict(base,local,latest), result = resolveTranscriptConflict(plan,{});
  assert.equal(plan.rows.length,1); assert.equal(plan.rows[0].suggested,"latest");
  assert.deepEqual(result.speaker_names,latest.speaker_names);
});
test("null name and removed override remain distinct explicit comparison values", () => {
  const base = payload(), local = payload(), latest = payload(); local.speaker_names = []; latest.speaker_names[0].display_name = null;
  const plan = planTranscriptConflict(base,local,latest);
  assert.equal(plan.rows[0].local,undefined); assert.equal(plan.rows[0].latest,null); assert.equal(plan.rows[0].conflict,true);
  assert.deepEqual(resolveTranscriptConflict(plan,choose(plan,"name","local")).speaker_names,[]);
  assert.deepEqual(resolveTranscriptConflict(plan,choose(plan,"name","latest")).speaker_names,[{ speaker_id: "s",display_name: null }]);
});
test("override removal and simultaneous text change cannot silently produce a partial utterance", () => {
  const base = payload(), local = payload(), latest = payload(); local.utterances = []; latest.utterances[0].effective_text = "最新変更";
  const plan = planTranscriptConflict(base,local,latest);
  assert.throws(() => resolveTranscriptConflict(plan,choose(plan,"text","latest")),/選択が一致/);
  const allLatest = Object.fromEntries(plan.rows.map(row => [row.key,"latest" as const]));
  assert.deepEqual(resolveTranscriptConflict(plan,allLatest).utterances,latest.utterances);
  const allLocal = Object.fromEntries(plan.rows.map(row => [row.key,"local" as const]));
  assert.deepEqual(resolveTranscriptConflict(plan,allLocal).utterances,[]);
});
test("manual speakers, merges and structural splits participate in comparison without automatic reapplication", () => {
  const base = empty(), local = empty(), latest = empty(); local.schema_version = latest.schema_version = 4;
  local.manual_speakers = [{ id: "manual:a",label: "手元追加" }]; latest.manual_speakers = [{ id: "manual:b",label: "最新追加" }];
  local.speaker_merges = [{ source_id: "s",target_id: "t" }]; latest.speaker_merges = [{ source_id: "s",target_id: "v" }];
  local.utterance_splits = [{ source_utterance_id: "u",source_effective_text: "手元分割",parts: [] }];
  latest.utterance_splits = [{ source_utterance_id: "u",source_effective_text: "最新分割",parts: [] }];
  const plan = planTranscriptConflict(base,local,latest);
  assert.equal(plan.rows.filter(row => row.conflict).length,2);
  const result = resolveTranscriptConflict(plan,{ ...choose(plan,"merge","latest"),...choose(plan,"split","local") });
  assert.deepEqual(result.manual_speakers,[...local.manual_speakers,...latest.manual_speakers]);
  assert.deepEqual(result.speaker_merges,latest.speaker_merges); assert.deepEqual(result.utterance_splits,local.utterance_splits);
  // Resolution is a pure comparison result, not native validation or a write.
  assert.deepEqual(latest.utterance_splits[0].parts,[]);
});
test("canonical sorting avoids false conflicts for reordered speaker IDs and rows", () => {
  const base = payload(), local = payload(), latest = payload();
  base.utterances[0].speaker_ids = ["s","t"]; local.utterances[0].speaker_ids = ["t","s"]; latest.utterances[0].speaker_ids = ["s","t"];
  const plan = planTranscriptConflict(base,local,latest);
  assert.deepEqual(plan.rows,[]); assert.deepEqual(resolveTranscriptConflict(plan,{}),canonicalPayload(base));
});
test("comparison snapshots and resolved copies cannot mutate original input or subsequent resolutions", () => {
  const base = payload(), local = payload(), latest = payload(); local.utterances[0].effective_text = "保持";
  const plan = planTranscriptConflict(base,local,latest); local.utterances[0].effective_text = "外部変更";
  assert.throws(() => { plan.local.utterances[0].effective_text = "改変"; },TypeError);
  const result = resolveTranscriptConflict(plan,{}); result.utterances[0].speaker_ids.push("foreign");
  assert.equal(resolveTranscriptConflict(plan,{}).utterances[0].effective_text,"保持");
  assert.deepEqual(resolveTranscriptConflict(plan,{}).utterances[0].speaker_ids,["s"]);
});
test("unknown choices, duplicate IDs and unsupported formats are rejected rather than silently ignored", () => {
  const plan = planTranscriptConflict(payload(),payload(),payload());
  assert.throws(() => resolveTranscriptConflict(plan,{ unknown: "local" }),/比較していない/);
  const duplicate = payload(); duplicate.speaker_names.push({ ...duplicate.speaker_names[0] });
  assert.throws(() => planTranscriptConflict(payload(),duplicate,payload()),/重複/);
  assert.throws(() => planTranscriptConflict({ ...payload(),schema_version: 99 },payload(),payload()),/未対応/);
});
test("large comparison keeps all independent changes with deterministic unambiguous keys", () => {
  const base = empty(), local = empty(), latest = empty();
  for(let i = 0; i < 5000; i++) {
    const entry = { utterance_id: `u:${i}`,effective_text: `原文${i}`,speaker_ids: ["s"] };
    base.utterances.push(entry); local.utterances.push({ ...entry,effective_text: `手元${i}` }); latest.utterances.push({ ...entry,speaker_ids: ["t"] });
  }
  const plan = planTranscriptConflict(base,local,latest), result = resolveTranscriptConflict(plan,{});
  assert.equal(plan.rows.length,10_000); assert.equal(new Set(plan.rows.map(row => row.key)).size,10_000);
  assert.equal(result.utterances.length,5000); assert.ok(result.utterances.every(row => row.effective_text.startsWith("手元") && row.speaker_ids[0] === "t"));
});
