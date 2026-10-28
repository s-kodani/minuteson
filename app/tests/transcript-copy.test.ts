import assert from "node:assert/strict";
import test from "node:test";
import { transcriptCopyText, writeTranscriptClipboard } from "../src/transcript-copy.ts";
import { projectTranscript, type Analysis, type DraftPayload } from "../src/transcript-state.ts";

const analysis: Analysis = {
  id: "a", meetingId: "m", contentHash: "h",
  speakers: [{ id: "s1", label: "話者A", displayName: "田中" }, { id: "s2", label: "話者B", displayName: null }],
  utterances: [
    { id: "u1", startUs: 5_900_000, endUs: 7_000_000, recognizedText: "原文", effectiveText: "原文", attributionState: "assigned", speakerIds: ["s1"] },
    { id: "u2", startUs: 3_661_000_000, endUs: 3_662_000_000, recognizedText: "次の発言", effectiveText: "次の発言", attributionState: "overlap", speakerIds: ["s1", "s2"] },
    { id: "u3", startUs: 3_663_000_000, endUs: 3_664_000_000, recognizedText: "不明", effectiveText: "不明", attributionState: "unknown", speakerIds: [] },
  ],
};

test("copy includes every projected utterance, edited names, text, assignments and timestamps", () => {
  const draft: DraftPayload = { schema_version: 4, speaker_names: [{ speaker_id: "s1", display_name: "山田" }],
    utterances: [{ utterance_id: "u1", effective_text: "修正した本文\n続き", speaker_ids: ["s2"] }] };
  assert.equal(transcriptCopyText(projectTranscript(analysis, draft)),
    "[00:00:05] 話者B：修正した本文\n続き\n[01:01:01] 山田、話者B：次の発言\n[01:01:03] 話者不明：不明");
  assert.equal(analysis.utterances[0].effectiveText, "原文");
});

test("empty transcripts and missing speaker references copy without invented names", () => {
  assert.equal(transcriptCopyText({ ...analysis, utterances: [] }), "");
  assert.equal(transcriptCopyText({ ...analysis, utterances: [{ ...analysis.utterances[0], speakerIds: ["missing"] }] }), "[00:00:05] 話者不明：原文");
});

test("clipboard starts within the gesture and receives text only after a delayed save; save failures never publish text", async () => {
  const descriptors = ["navigator", "ClipboardItem"].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  let started = false, copied: string | null = null;
  class Item {
    readonly data: Record<string, Promise<Blob>>;
    constructor(data: Record<string, Promise<Blob>>) { this.data = data; }
  }
  Object.defineProperty(globalThis, "ClipboardItem", { configurable: true, value: Item });
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { write: async ([item]: Item[]) => {
    started = true;
    const blob = await item.data["text/plain"];
    copied = await blob.text();
  } } } });
  try {
    let saved!: (text: string) => void;
    const operation = writeTranscriptClipboard(() => new Promise(resolve => { saved = resolve; }));
    assert.equal(started, true);
    assert.equal(copied, null);
    saved("保存済みの本文"); await operation;
    assert.equal(copied, "保存済みの本文");
    copied = null;
    await assert.rejects(writeTranscriptClipboard(async () => { throw Error("保存失敗"); }), /保存失敗/);
    assert.equal(copied, null);
    Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { write: async () => { throw Error("権限なし"); } } } });
    await assert.rejects(writeTranscriptClipboard(async () => "本文"), /権限なし/);
  } finally {
    for (const [key, descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
