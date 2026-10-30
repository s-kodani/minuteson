import assert from "node:assert/strict";
import test from "node:test";
import { minutesCopyText } from "../src/minutes-copy.ts";

test("minutes copy preserves Markdown while replacing audio links with whole-second starting times", () => {
  const source = "# 議事録\n\n## 要約\n**設計**を確認した。[根拠](audio:00:10:00.987654-00:10:11)\n\n- 次回の予定。[01:02:03–01:02:05](audio:01:02:03-01:02:05)\n- 別の根拠。[再生](audio:123:45:56-123:46:00)\n";
  assert.equal(minutesCopyText(source), "# 議事録\n\n## 要約\n**設計**を確認した。(00:10:00)\n\n- 次回の予定。(01:02:03)\n- 別の根拠。(123:45:56)\n");
  assert.ok(source.includes("audio:00:10:00.987654"));
});

test("ordinary links, invalid audio intervals, escaped text and code examples stay exact", () => {
  const source = [
    "[資料](https://example.com) [同じ時刻](audio:00:01:00-00:01:00) [不正](audio:00:61:00-00:62:00)",
    "`[例](audio:00:01:00-00:01:20)` と \\[例](audio:00:01:00-00:01:20)",
    "```markdown", "[例](audio:00:01:00-00:01:20)", "```",
    "    [例](audio:00:01:00-00:01:20)", "",
  ].join("\n");
  assert.equal(minutesCopyText(source), source);
  assert.equal(minutesCopyText(""), "");
});

test("audio links with formatting, Unicode and reference syntax use parser source offsets", () => {
  assert.equal(minutesCopyText("🦀[**根拠**](audio:00:00:01-00:00:03) [再生][a]\n\n[a]: audio:00:00:04-00:00:05\n"),
    "🦀(00:00:01) (00:00:04)\n\n\n");
});

test("reference definitions survive when an image still uses them; unrelated definitions stay exact", () => {
  assert.equal(minutesCopyText("[根拠][a] ![画像][a]\n\n[a]: audio:00:00:04-00:00:05\n[b]: https://example.com\n"),
    "(00:00:04) ![画像][a]\n\n[a]: audio:00:00:04-00:00:05\n[b]: https://example.com\n");
  assert.equal(minutesCopyText("[a]: audio:00:00:04-00:00:05\n\n[根拠][a]\n"), "\n\n(00:00:04)\n");
});
