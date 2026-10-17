import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const panel = readFileSync(new URL("../src/minutes.tsx", import.meta.url), "utf8");
test("minutes has no per-meeting model selector or profile state", () => {
  assert.doesNotMatch(panel, /生成に使うモデル|setProfile|highAccuracy|high_accuracy|<select/);
  assert.match(panel, /const engine = settings\?\.standard/);
});
test("new generation reads the global minutes engine at confirmed admission", () => {
  const enqueue = panel.slice(panel.indexOf("const prepare = async"), panel.indexOf("if (saved) await confirm"));
  assert.match(enqueue, /await invoke<VersionedDraft<MinutesPreferences>>\("get_minutes_preferences"\)/);
  assert.match(enqueue, /const engine = preferences\.payload\.standard/);
  assert.match(enqueue, /minutesConfigured\(engine\)/);
  assert.match(enqueue, /\.\.\.engine, profile: "standard"/);
  assert.match(panel, /action: prepare/);
  assert.match(panel, /旧版は保存しません。失敗した場合は現在の本文を保持します/);
});
