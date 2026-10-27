import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ts from "typescript";

const require = createRequire(import.meta.url);
function component(name: string): any {
  const source = readFileSync(new URL(`../src/${name}.tsx`, import.meta.url), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const module = { exports: {} };
  new Function("require", "module", "exports", compiled)(require, module, module.exports);
  return module.exports;
}
const { AudioDeletionConfirmation } = component("audio-deletion-confirmation");
const { MeetingDeletionMenu } = component("meeting-deletion-menu");

test("audio confirmation shows the meeting, retained text and exactly cancel/delete", () => {
  const props = { name: "長い会議名 <検証>", heading: { current: null }, blocked: false, canDelete: true, onCancel() {}, onDelete() {} };
  const render = (patch = {}) => renderToStaticMarkup(createElement(AudioDeletionConfirmation, { ...props, ...patch }));
  const html = render();
  assert.match(html, /音声だけを削除/u);
  assert.match(html, /長い会議名 &lt;検証&gt;/u);
  assert.match(html, /音声は復元できません。文字起こし・議事録・根拠テキストと履歴は保持します。/u);
  assert.equal((html.match(/<button\b/gu) ?? []).length, 2);
  assert.match(html, />キャンセル<\/button>[\s\S]*>削除<\/button>/u);
  assert.doesNotMatch(html, /<input|<ul|disabled=""/u);
  assert.equal((render({ blocked: true }).match(/disabled=""/gu) ?? []).length, 2);
  assert.equal((render({ canDelete: false }).match(/disabled=""/gu) ?? []).length, 1);
});

test("audio menu exposes hover help outside a disabled item and keeps meeting deletion available", () => {
  const props = {
    meeting: { id: "meeting", title: "対象", lifecycle: "recorded" },
    deletion: { canReviewMeetingId: () => true, meetingReviewReason: () => null, meetingBlockers: () => [], retryAvailability: async () => {}, reviewMeeting() {} },
  };
  for (const reason of [null, "音声はすでに削除済みです。", "音声の削除状態を確認しています。", "音声の削除状態を確認できません。"] as const) {
    const html = renderToStaticMarkup(createElement(MeetingDeletionMenu, { ...props, audioDeletion: { canReviewNewId: () => reason === null, meetingReviewReason: () => reason, reviewNew() {} } }));
    assert.match(html, /aria-label="対象を削除">削除<\/button>/u);
    assert.equal((html.match(/disabled=""/gu) ?? []).length, reason ? 1 : 0);
    if (reason) {
      assert.ok(html.includes(`class="audio-deletion-menu-item" title="${reason}"><button`));
      assert.match(html, /aria-describedby=/u);
    }
  }
});
