import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");

test("model selection exposes one row per purpose and inline installed inventory", () => {
  const view = source("model-inspection.tsx");
  for (const label of ["文字起こし", "話者分離", "議事録生成"]) assert.ok(view.includes('label: "' + label + '"'));
  assert.match(view, /モデルを選択/);
  assert.match(view, /available \? "使用" : "ダウンロードして使用"/);
  assert.match(view, /aria-label="導入済みモデル"/);
  assert.doesNotMatch(view, /<ModelDownloadPanel|<ModelInventory|<SettingsDisclosure|取得記録|取得定義からダウンロード|モデルの配置・構成記録/);
});
test("one action verifies download, placement, CPU and saved global selection", () => {
  const view = source("model-inspection.tsx");
  for (const check of ["assertDownloadReceipt", "assertPlacementReceipt", "assertCompatibilitySummary", "assertApplicationPreview", "assertApplicationResult"]) assert.ok(view.includes(check + "("));
  assert.match(view, /profile: "standard"/);
  assert.match(view, /get_model_download_catalog/);
  assert.match(view, /read_model_download/);
  assert.match(view, /downloads.set/);
  assert.match(view, /sourceType === "download" && item.download\?\.recipeHash === option.review.recipeHash/);
});
test("uncertain model writers are retained until cancellation and durable readback", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /let unresolved: Pending \| null = null/);
  assert.match(view, /retain\(\{ token, stage, \.\.\.context \}\)/);
  assert.match(view, /lifetime.active \|\| !lifetime.cancelled/);
  assert.match(view, /item.packageId === value.token && item.placementRecorded/);
  assert.match(view, /item.checkOperationId === value.preview\?\.checkOperationId && item.settingsMatch && item.compatibilityCurrent/);
  const recheck = view.slice(view.indexOf("const recheck"), view.indexOf("const disabled"));
  assert.doesNotMatch(recheck, /start_model_download|place_downloaded_model|check_model_compatibility|apply_model_profile_application/);
});
test("local runtime execution requires explicit trust even for previously imported files", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /local.sourceType !== "download" && !canReviewApplication\(local\)/);
  assert.match(view, /setLocalReview\(local\); setTrusted\(false\)/);
  assert.match(view, /!source \|\| !report \|\| !trusted \|\| unresolved/);
  assert.match(view, /disabled=\{disabled \|\| !trusted\}/);
});
test("simple model rows retain aligned controls and mobile wrapping", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /\.model-selection-row \{ display:flex; align-items:flex-end; gap:12px/);
  assert.match(css, /\.model-selection-row>button,\.model-selection-row select \{ min-height:40px;/);
  assert.match(css, /\.model-selection-row \{ flex-wrap:wrap/);
});
test("shared settings modal presentation never writes in an effect", () => {
  const view = source("settings-disclosure.tsx"), modal = source("options-modal.tsx");
  assert.match(view, /<OptionsModal/);
  assert.doesNotMatch(view, /useEffect|invoke\(|\.flush\(/);
  assert.match(modal, /aria-haspopup="dialog"/);
});
