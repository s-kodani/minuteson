import { test } from "node:test";
import assert from "node:assert/strict";
import { checkedAppUpdateStatus, updateHeading, updateInstructions, type AppUpdateStatus } from "../src/app-update-state.ts";
const state = (patch: Partial<AppUpdateStatus> = {}): AppUpdateStatus => ({ revision:1,channel:"windows_store",state:"available",currentVersion:"1.0.1",updateAvailable:true,availableVersion:null,checkedAtMs:10,errorMessage:null,...patch });
test("checks schema and availability without claiming an installed or downloaded version", () => {
  assert.equal(updateHeading(checkedAppUpdateStatus(state())), "Microsoft Storeに更新があります");
  assert.match(updateHeading(state({channel:"macos_release",availableVersion:"1.0.2"})), /1\.0\.2/);
  assert.match(updateHeading(state({state:"no_update",updateAvailable:false})), /見つかりません/);
  assert.match(updateHeading(state({state:"unavailable",errorMessage:"offline"})), /確認できません/);
  assert.throws(() => checkedAppUpdateStatus({}));
  assert.throws(() => checkedAppUpdateStatus(state({revision:NaN})));
  assert.throws(() => checkedAppUpdateStatus(state({state:"no_update"})));
  assert.throws(() => checkedAppUpdateStatus(state({channel:"unsupported"})));
  assert.match(updateInstructions(state()), /終了/);
  assert.match(updateInstructions(state({channel:"macos_release"})), /入れ替え/);
});
