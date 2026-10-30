import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync,existsSync} from "node:fs";
test("test-audio cleanup has no UI, command, navigation lock or exit confirmation",()=>{
  const root=new URL("../",import.meta.url);
  const main=readFileSync(new URL("src/main.tsx",root),"utf8");
  const ipc=readFileSync(new URL("src-tauri/src/lib.rs",root),"utf8");
  assert.doesNotMatch(main,/CaptureTestRecovery|captureTestRecovery|未回収のテスト音声|テスト音声の回復/);
  assert.doesNotMatch(ipc,/get_capture_test_recovery|retry_capture_test_recovery/);
  for(const file of ["src/capture-test-recovery.tsx","src/capture-test-recovery-client.ts"])assert.equal(existsSync(new URL(file,root)),false);
  const controller=readFileSync(new URL("src-tauri/src/capture_test.rs",root),"utf8");
  assert.match(controller,/mon-test-audio-recovery/);
  assert.match(controller,/state\.current\.is_none\(\)/);
  assert.match(controller,/state\.ownership\.retry_revision\(\)/);
  assert.match(controller,/Self::recover\(&handle, Some\(revision\)\)/);
  assert.match(controller,/Duration::from_secs\(30\)/);
});
test("failure-job retries and landing management dialogs are not app features",()=>{
  const src=name=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");
  assert.doesNotMatch(src("minutes.tsx"),/OptionsModal|retry_minutes_generation|retryOperations|議事録の操作/);
  assert.match(src("minutes.tsx"),/>議事録を生成<\/button>/);
  assert.doesNotMatch(src("analysis.tsx"),/retry_transcription|retry_diarization|retryOperations|>再試行<\/button>/);
  assert.doesNotMatch(src("main.tsx"),/会議管理・録音・解析操作|renderMeetingTools/);
  assert.doesNotMatch(src("main.tsx"),/landing-tools|その他の操作/);
  assert.doesNotMatch(src("main.tsx"),/const createMeeting|音声なしの会議を作成/);
});
