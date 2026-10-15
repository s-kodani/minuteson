import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { assertUiPreferences } from "../src/theme-state.ts";
const source = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");

test("scratch registry has no old version upgrade and keeps root and schema proofs", () => {
  const registry = source("../src-tauri/src/storage/backup/scratch.rs");
  assert.doesNotMatch(registry, /upgrade_old|upgrade_two|upgrade_three|previous_schema|version_two_schema|version_three_schema/);
  assert.match(registry, /version != local_version\(\) \|\| schema\(&db\)\? != expected_schema/);
  assert.match(registry, /saved != root_identity/);
  assert.match(registry, /registry\.recover\(\)\?/);
});
test("theme accepts current color/brightness preferences and refuses removed reading controls", () => {
  assert.throws(() => assertUiPreferences({id:"ui_preferences",lockVersion:1,payload:{schemaVersion:2,theme:"dark",textSize:15,followPlayback:false}}));
  for (const file of ["../src/theme-state.ts", "../src/theme.tsx", "../src/meeting-audio.tsx"]) assert.doesNotMatch(source(file), /pendingDisplay|changeDisplay|savePayload|TextSizeSelection|PlaybackFollowingSelection|readingPreferences/);
  assert.doesNotMatch(source("../src-tauri/src/storage/ui_preferences.rs"), /V2|text_size|follow_playback/);
});
test("playback highlight cannot change scroll focus or pages", () => {
  assert(!existsSync(new URL("../src/playback-following.tsx", import.meta.url)));
  assert.doesNotMatch(source("../src/playback-highlights.tsx"), /scrollTop|setPage|\.focus\(|useTheme/);
  assert.doesNotMatch(source("../src/transcript.tsx"), /usePlaybackFollowing|following\.suspend|following\.handlers/);
});
