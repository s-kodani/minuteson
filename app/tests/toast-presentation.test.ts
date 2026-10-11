import test from "node:test";
import assert from "node:assert/strict";
import { notificationCapacity, removedPresentationIds, presentationId, stackCardExposed } from "../src/toast-presentation.ts";
import { ToastController } from "../src/toast-state.ts";

test("visual removal only selects exact old event ids, not semantic acknowledgement", () => {
  assert.deepEqual(removedPresentationIds(new Set([presentationId(1), presentationId(2)]), [{ id: 2 }, { id: 3 }]), [presentationId(1)]);
  assert.deepEqual(removedPresentationIds(new Set([presentationId(2)]), [{ id: 2 }]), []);
});
test("a collapsed rear, exiting or overflow notice is not a presentation receipt", () => {
  assert.equal(stackCardExposed({ expanded: "false", front: "true", mounted: "true" }), true);
  assert.equal(stackCardExposed({ expanded: "false", front: "false" }), false);
  assert.equal(stackCardExposed({ expanded: "true", front: "false" }), true);
  for (const state of [{ visible: "false" }, { removed: "true" }, { mounted: "false" }]) assert.equal(stackCardExposed(state), false);
});
test("short windows reduce visible slots without creating a notification scrollport", () => {
  assert.equal(notificationCapacity(464), 3);
  assert.equal(notificationCapacity(300), 2);
  assert.equal(notificationCapacity(144), 1);
  assert.equal(notificationCapacity(10), 1);
  assert.equal(notificationCapacity(400, 160), 2);
});
test("stack pauses and visual eviction preserve queued important notices and safe actions", async () => {
  const c = new ToastController(); let receipts = 0, actions = 0, acknowledged = 0;
  for (let i = 0; i < 4; i++) c.show({ key: `important-${i}`, level: "error", message: `問題${i}`, onRendered: () => receipts++, onClose: () => acknowledged++, action: { kind: "retry", label: "再試行", run: () => { actions++; } } });
  assert.equal(c.getSnapshot().length, 3); assert.equal(c.pendingCount, 1);
  const first = c.getSnapshot()[0];
  c.pause(first.key, "stack-hover", true, first.id);
  c.markRendered(first.key, first.id); assert.equal(receipts, 1);
  await c.act(first.key, first.id); assert.equal(actions, 1);
  c.dismiss(first.key, first.id); assert.equal(acknowledged, 1);
  assert.equal(c.pendingCount, 0); assert.equal(c.getSnapshot().length, 3);
  c.show({ key: first.key, level: "error", message: first.message });
  assert.equal(c.getSnapshot().some(t => t.key === first.key), false);
  for (const notice of c.getSnapshot()) c.resolve(notice.key);
});
