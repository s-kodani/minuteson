import test from "node:test";
import assert from "node:assert/strict";
import { assertStorageView, type StorageView } from "../src/storage-placement-state.ts";
const view = (): StorageView => ({ meetingRoot: "/new", activeMeetingRoot: "/old", restartRequired: true, modelsRoot: "/models", bootstrapPath: "/config/bootstrap.json", revision: 2, origin: "relocated", pendingOperationId: null, pendingTarget: null, keptMoves: [] });
test("storage selection differs from active root only with explicit restart obligation", () => {
  assert.doesNotThrow(() => assertStorageView(view()));
  assert.throws(() => assertStorageView({ ...view(), restartRequired: false }));
  assert.doesNotThrow(() => assertStorageView({ ...view(), meetingRoot: "/old" }));
  for (const change of [{ activeMeetingRoot: "relative" }, { revision: -1 }, { pendingTarget: "/partial" }, { pendingOperationId: "forged" }]) assert.throws(() => assertStorageView({ ...view(), ...change }));
});
