import test from "node:test";
import assert from "node:assert/strict";
import { publishWithReplacementConfirmation, type Publication, type ReplacementTarget } from "../src/transcription-regeneration.ts";
import { ConfirmationController } from "../src/confirmation-state.ts";
import type { AnalysisFlow } from "../src/analysis-flow-state.ts";
const flow: AnalysisFlow = { id: "flow", meetingId: "meeting", lockVersion: 3, stage: "diarization", transcriptionJob: null, diarizationJob: null, errorMessage: null };
const target: ReplacementTarget = { analysisId: "original", contentHash: "a".repeat(64), draft: { id: "draft", lockVersion: 2, contentHash: "b".repeat(64) } };
const pending: Publication = { status: "confirmationRequired", flow, target, meetingTitle: "会議A" };
const published: Publication = { status: "published", flow: { ...flow, stage: "review" } };

test("edited regeneration waits for explicit consent and publishes the same result after another save barrier", async () => {
  const events: string[] = [];
  const controller = new ConfirmationController();
  const operation = publishWithReplacementConfirmation(flow, {
    saveThenAct: async action => { events.push("save"); await action(); events.push("unlock"); },
    publish: async (current, confirmed) => { assert.equal(current.id, flow.id); events.push(confirmed ? "replace" : "check"); if (confirmed) assert.deepEqual(confirmed, target); return confirmed ? published : pending; },
    confirm: options => { assert.equal(events.at(-1), "unlock"); assert.equal(options.confirmLabel, "置き換える"); assert.match(options.target, /会議A/); return controller.request({}, null, null, options); },
  });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(events, ["save", "check", "unlock"]);
  const id = controller.getSnapshot()!.id;
  await Promise.all([controller.execute(id), controller.execute(id)]);
  assert.equal(await operation, "completed");
  assert.deepEqual(events, ["save", "check", "unlock", "save", "replace", "unlock"]);
});
test("cancelling replacement leaves generated output and current draft untouched", async () => {
  let calls = 0;
  const controller = new ConfirmationController();
  const operation = publishWithReplacementConfirmation(flow, {
    saveThenAct: async action => action(), publish: async () => { calls++; return pending; },
    confirm: options => controller.request({}, null, null, options),
  });
  await new Promise(resolve => setImmediate(resolve));
  controller.cancel(controller.getSnapshot()!.id);
  assert.equal(await operation, "cancelled"); assert.equal(calls, 1);
});
test("a failed save on acceptance never sends the replacement command", async () => {
  let saves = 0, calls = 0;
  const controller = new ConfirmationController();
  const operation = publishWithReplacementConfirmation(flow, {
    saveThenAct: async action => { if (++saves === 2) throw Error("save failed"); await action(); },
    publish: async () => { calls++; return pending; }, confirm: options => controller.request({}, null, null, options),
  });
  await new Promise(resolve => setImmediate(resolve));
  await controller.execute(controller.getSnapshot()!.id);
  assert.equal(controller.getSnapshot()!.phase, "failed"); assert.equal(calls, 1);
  controller.cancel(controller.getSnapshot()!.id);
  assert.equal(await operation, "unconfirmed");
});
test("an unchanged transcript publishes automatically without a second modal", async () => {
  let confirms = 0;
  assert.equal(await publishWithReplacementConfirmation(flow, {
    saveThenAct: async action => action(), publish: async () => published,
    confirm: async () => { confirms++; return "cancelled"; },
  }), "completed");
  assert.equal(confirms, 0);
});
