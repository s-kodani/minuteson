import test from "node:test";
import assert from "node:assert/strict";
import { assertStorageView, type StorageView } from "../src/storage-placement-state.ts";
const view = (): StorageView => ({ meetingRoot: "/new", activeMeetingRoot: "/old", restartRequired: true, modelsRoot: "/models", bootstrapPath: "/config/bootstrap.json", revision: 2, origin: "relocated", pendingOperationId: null, pendingTarget: null, keptMoves: [] });
test("storage selection differs from active root only with explicit restart obligation", () => {
  assert.doesNotThrow(() => assertStorageView(view()));
  assert.throws(() => assertStorageView({ ...view(), restartRequired: false }));
  assert.doesNotThrow(() => assertStorageView({ ...view(), meetingRoot: "/old" }));
  assert.throws(() => assertStorageView({ ...view(), origin: "legacy" } as unknown as StorageView));
  assert.doesNotThrow(() => assertStorageView({ ...view(), origin: "default" }));
  for (const change of [{ activeMeetingRoot: "relative" }, { revision: -1 }, { pendingTarget: "/partial" }, { pendingOperationId: "forged" }]) assert.throws(() => assertStorageView({ ...view(), ...change }));
});
test("recovered storage includes a validated recovery result", () => {
  const recovered = { ...view(), origin: "recovered" as const, recovery: { kind: "device_changed" as const, reviewRequired: false, reasons: [], recordPath: "/config/recovery/previous-bootstrap.json" } };
  assert.doesNotThrow(() => assertStorageView(recovered));
  assert.throws(() => assertStorageView({ ...recovered, recovery: null }));
  assert.throws(() => assertStorageView({ ...recovered, recovery: { ...recovered.recovery, reviewRequired: true } }));
});
