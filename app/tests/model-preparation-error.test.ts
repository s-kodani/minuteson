import test from "node:test";
import assert from "node:assert/strict";
import { modelPreparationError } from "../src/model-preparation-error.ts";
import { normalizeAppError } from "../src/app-error.ts";

test("model file failures ask only for a retry after automatic reconciliation", () => {
  for (const detail of [
    "モデルの親フォルダーにリンクまたは変更があります",
    "検査中にモデルの親フォルダーが変更されました",
    "検査中にモデルパッケージが変更されました",
  ]) {
    const native = { code: "operation_result_unknown", message: detail, operationId: null, retryDisposition: "reconcile" };
    for (const cause of [detail, new Error(detail), native, normalizeAppError(native)]) {
      assert.equal(modelPreparationError(cause), "モデルを読み込めません。もう一度お試しください。");
    }
    assert.equal(native.message, detail, "display formatting must not modify diagnostics or recovery metadata");
  }
});

test("other model errors retain their guidance without an Error prefix", () => {
  const message = "使用するモデルを選択してください。";
  assert.equal(modelPreparationError(new Error(message)), message);
  assert.equal(modelPreparationError(message), message);
});
