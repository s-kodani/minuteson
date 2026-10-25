import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");

test("model selection exposes one row per purpose and separate installed model management", () => {
  const view = source("model-inspection.tsx");
  for (const label of ["文字起こし", "話者分離", "議事録生成"]) assert.ok(view.includes('label: "' + label + '"'));
  assert.match(view, /モデルを選択/);
  assert.match(view, /modelChoiceAction\(choice, entries, used\)/);
  assert.match(view, /in_use: "使用中"/);
  assert.match(view, /download: "ダウンロードして使用"/);
  assert.match(view, /aria-label="導入済みモデル管理"/);
  assert.doesNotMatch(view, /<ModelDownloadPanel|<ModelInventory|<SettingsDisclosure|取得記録|取得定義からダウンロード|モデルの配置・構成記録/);
});
test("one action verifies download, placement, CPU and saved global selection", () => {
  const view = source("model-inspection.tsx");
  for (const check of ["assertDownloadReceipt", "assertPlacementReceipt", "assertCompatibilitySummary", "assertApplicationPreview", "assertApplicationResult"]) assert.ok(view.includes(check + "("));
  assert.match(view, /profile: "standard"/);
  assert.match(view, /get_model_download_catalog/);
  assert.match(view, /read_model_download/);
  assert.match(view, /downloads.set/);
  assert.match(view, /sourceType === "download" && canReuseOfficialModel\(item, option\)/);
});
test("uncertain model writers are retained until cancellation and durable readback", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /let unresolved: Pending \| null = null/);
  assert.match(view, /retain\(\{ token, stage, \.\.\.context, startedAtMs: startedAt.current! \}\)/);
  assert.match(view, /lifetime.active \|\| !lifetime.cancelled/);
  assert.match(view, /item.packageId === value.token && item.reviewHash === download.report.reviewHash && item.placementRecorded/);
  assert.match(view, /assertDownloadCopyResult\(await invoke\("get_downloaded_model_placement_result"/);
  assert.match(view, /result.active \|\| !result.cancelled/);
  assert.match(view, /item.checkOperationId === value.preview\?\.checkOperationId && item.settingsMatch && item.compatibilityCurrent/);
  const recheck = view.slice(view.indexOf("const recheck"), view.indexOf("const disabled"));
  assert.doesNotMatch(recheck, /start_model_download|place_downloaded_model|check_model_compatibility|apply_model_profile_application/);
});
test("local import is removed and unverified installed runtimes cannot be executed", () => {
  const view = source("model-inspection.tsx");
  assert.doesNotMatch(view, /ModelImportGuide|ローカルから取り込む|place_model_package|inspect_model_package|<CommonModal/);
  assert.match(view, /disabled=\{!item.option && !!item.entry && item.entry.sourceType !== "download" && !canReviewApplication\(item.entry\)\}/);
  assert.match(view, /entry.sourceType !== "download" && !canReviewApplication\(entry\)\) throw Error/);
  const native=readFileSync(new URL("../src-tauri/src/lib.rs",import.meta.url),"utf8");
  assert.doesNotMatch(native,/inspect_model_package|select_model_files|place_model_package|mod model_file_dialog/);
});
test("simple model rows retain aligned controls and mobile wrapping", () => {
  const css = source("prototype-alignment.css");
  assert.match(css, /\.model-selection-row \{ display:flex; align-items:flex-end; gap:12px/);
  assert.match(css, /\.model-selection-row>button,\.model-selection-row select \{ min-height:40px;/);
  assert.match(css, /\.model-selection-row \{ flex-wrap:wrap/);
});
