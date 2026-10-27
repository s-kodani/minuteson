import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { renderToStaticMarkup } from "react-dom/server";
import { assertDeletionPreview, type DeletionPreview } from "../src/deletion-state.ts";
import { dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";

// Render the actual TSX component without a browser or a running native app.
const bundle = await build({
  stdin: { contents: readFileSync(new URL("../src/directory-deletion-confirmation.tsx", import.meta.url), "utf8"), loader: "tsx", resolveDir: fileURLToPath(new URL("../src/", import.meta.url)) },
  bundle: true, write: false, platform: "node", format: "esm", jsx: "automatic",
});
const { DirectoryDeletionConfirmation } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);
const props = { name: "長い会議名 <検証>", directory: "C:\\日本語の保存先\\meetings\\対象", titleId: "delete-title", heading: { current: null }, blocked: false, canDelete: true, onCancel() {}, onDelete() {} };

test("meeting and model confirmations show the requested content and exactly two buttons", () => {
  for (const kind of ["meeting", "model"]) {
    const html = renderToStaticMarkup(DirectoryDeletionConfirmation({ ...props, kind }));
    assert.equal((html.match(/<button\b/gu) ?? []).length, 2);
    assert.match(html, />キャンセル<\/button>[\s\S]*>削除<\/button>/u);
    assert.match(html, kind === "meeting" ? /会議を削除/u : /モデルを削除/u);
    assert.match(html, /長い会議名 &lt;検証&gt;/u);
    assert.ok(html.includes(props.directory));
    assert.match(html, /overflow-wrap:anywhere/u);
    assert.doesNotMatch(html, /<input|<ul|同意|再同意/u);
    assert.doesNotMatch(html, /disabled=""/u);
    const blocked = renderToStaticMarkup(DirectoryDeletionConfirmation({ ...props, kind, blocked: true }));
    assert.equal((blocked.match(/disabled=""/gu) ?? []).length, 2);
  }
});

const uuid = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const preview = (): DeletionPreview => ({ scopeMode: "directory", targetDirectory: "/private/data/meetings/target", confirmationId: uuid(1), operationId: uuid(2), meetingId: uuid(3), title: "対象", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, pathCount: 1, processedPaths: 0, fileCount: 0, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: 0, rowCount: 5, rowPhaseCommitted: false });

test("directory confirmation requires a native absolute target and sends IDs only", () => {
  const p = preview();
  assert.doesNotThrow(() => assertDeletionPreview(p));
  for (const targetDirectory of [undefined, "relative/folder", "/bad\0path"]) assert.throws(() => assertDeletionPreview({ ...p, targetDirectory }));
  assert.doesNotThrow(() => assertDeletionPreview({ ...p, targetDirectory: props.directory }));
  const s: DataSnapshot = { revision: 1, token: uuid(4), kind: "deletion", phase: "ready", busy: false, backupPreview: null, restorePreview: null, relocationPreview: null, backupReceipt: null, restoreReceipt: null, relocationReceipt: null, deletionPreview: p, deletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true };
  assert.deepEqual(dataRequest(s, "publish"), { token: uuid(4), confirmationId: p.confirmationId, action: "publish" });
  assert.throws(() => dataRequest({ ...s, busy: true }, "publish"));
});
