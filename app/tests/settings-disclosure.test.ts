import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");

test("settings keep mounted modal inputs and attention never opens a background modal", () => {
  const view = source("settings-disclosure.tsx"), modal = source("options-modal.tsx");
  assert.match(view, /<OptionsModal label=\{label\} className=\{`settings-disclosure \$\{className\}`\} attention=\{attention\}[^\n]*>\{children\}/);
  assert.doesNotMatch(view, /useEffect|setOpen|disabled=\{attention\}/);
  assert.match(modal, /aria-haspopup="dialog"/);
  assert.match(modal, /<CommonModal[^\n]*open=\{open && !hidden\} nested data-settings-modal/);
  assert.doesNotMatch(modal, /open && children|flush\(|invoke\(/);
});

test("model settings expose only two onboarding entries and keep portaled spacing", () => {
  const local = source("model-inspection.tsx"), download = source("model-download.tsx"), css = source("prototype-alignment.css");
  assert.equal((local.match(/<SettingsDisclosure/g) ?? []).length, 1);
  assert.equal((download.match(/<SettingsDisclosure/g) ?? []).length, 1);
  assert.match(local, /label="ローカルから取り込む" className="model-inspection-settings"/);
  assert.match(download, /label="モデルをダウンロード" className="model-download-settings"/);
  assert.doesNotMatch(local + download, /<CommonModal|label="取得定義からダウンロード"|label="取得記録"|label="モデルの配置・構成記録"|reviewDeletion|reviewNew\(/);
  assert.match(css, /\.model-inspection-settings \.options-dialog-content>section \{\s*display:grid; gap:12px;/);
  assert.match(css, /\.model-download-settings \.model-download-catalog \{[^}]*max-height:none; overflow:visible;/);
  assert.match(css, /\.model-setup-entrypoints \{[^}]*align-items:center;[^}]*gap:12px/);
});

test("normal settings retain theme but show installed models inline, not record/help modals", () => {
  const main = source("main.tsx"), local = source("model-inspection.tsx"), inventory = source("model-inventory.tsx");
  assert.match(main, /<ThemeSettingsPanel compact locked/);
  assert.match(main, /<ModelInspectionPanel compact locked/);
  assert.match(local, /<ModelInventory entries=\{placements\} profiles=\{profiles\} loading=\{catalogLoading\} error=\{catalogError\}/);
  assert.doesNotMatch(local, /InfoPopover|CommonModal/);
  assert.doesNotMatch(source("model-issue.tsx"), /InfoPopover|CommonModal|<button/);
  assert.match(source("model-issue.tsx"), /role="alert"/);
  assert.match(inventory, /aria-label="導入済みモデル"/);
  assert.match(inventory, /モデル一覧を確認できません/);
  assert.match(inventory, /!loading && !error && loaded.length === 0/);
});

test("download review, receipt and uncertainty remain in the initiating onboarding dialog", () => {
  const view = source("model-download.tsx");
  const dialog = view.slice(view.indexOf('<SettingsDisclosure'), view.indexOf('</SettingsDisclosure>'));
  for (const label of ["未確定のモデル取得操作", "照合済み取得素材", "ダウンロードと取り込みの確認"]) assert.ok(dialog.includes(`aria-label="${label}"`));
  assert.match(dialog, /未完了・過去のダウンロード/);
  assert.match(view, /await invoke\("cancel_model_inspection", \{ token: value.token \}\)/);
  assert.match(view, /if \(lifetime.active \|\| !lifetime.cancelled\) throw Error/);
  assert.match(view, /自動再取得は行いません/);
});

test("manual copy and CPU execution need explicit actions but never additional confirmation modals", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /assertGuidedInspection\(result, source\)/);
  assert.match(view, /onClick=\{\(\) => void place\(\)\}>取り込む/);
  assert.match(view, /setReport\(null\); reviewSource.current = null;/);
  assert.match(view, /trustedPackage !== selected.packageId/);
  assert.match(view, /current.current.trustedPackage !== entry.packageId/);
  assert.match(view, /最大120秒/);
  assert.match(view, /隔離する仕組みはありません/);
  assert.match(view, /setTrustedPackage\(""\)/);
  assert.match(view, /if \(!\(locked \|\| recordingActive\) \|\| !running.current\) return/);
  assert.match(view, /cancel_model_inspection.*, \{ token \}/);
  const inspect = view.slice(view.indexOf('const inspectSource ='), view.indexOf('const place ='));
  assert.doesNotMatch(inspect, /check_model_compatibility|apply_model_profile_application|place_model_package/);
});

test("controlled settings pass presentation and explicit restoration without save effects", () => {
  const view = source("settings-disclosure.tsx"), modal = source("options-modal.tsx");
  assert.match(view, /open=\{open\} onOpenChange=\{onOpenChange\} triggerRef=\{triggerRef\} restoreTarget=\{restoreTarget\}/);
  assert.match(modal, /<CommonModal[^\n]*restoreTarget=\{restoreTarget\}/);
  assert.doesNotMatch(view, /useEffect|invoke\(|\.flush\(/);
});

test("download readback is read-only and exposes a next step without copy, execute or apply", () => {
  const view = source("model-download.tsx");
  const inspect = view.slice(view.indexOf('const inspectEntry ='), view.indexOf('const supported ='));
  assert.match(inspect, /result.state === "complete" && recordsVisible.current/);
  assert.match(inspect, /setRecordsFeedback\(feedback\)/);
  assert.match(inspect, /setRecordsError\(String\(cause\)\)/);
  assert.doesNotMatch(inspect, /start_model_download|place_downloaded_model|check_model_compatibility|apply_model|copy\(\)|start\(\)/);
});

test("completed inline download readback restores only its owned visible next action", () => {
  const view = source("model-download.tsx");
  const restore = view.slice(view.indexOf('useLayoutEffect(() =>'), view.indexOf('const current ='));
  assert.match(restore, /officialOpen && recordsVisible.current && source.isConnected && document.activeElement === source/);
  assert.match(restore, /!target.disabled && !target.closest\("\[hidden\], \[inert\]"\)/);
  assert.match(restore, /target.focus\(\{ preventScroll: true \}\)/);
  assert.match(restore, /target.scrollIntoView\(\{ block: "nearest", inline: "nearest" \}\)/);
  assert.doesNotMatch(restore, /invoke\(|start\(|copy\(/);
});

test("same application operation survives uncertain results and can be explicitly released after fencing and fresh read", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /assertApplicationPreview\(preview, entry, name, profile\)/);
  assert.match(view, /assertApplicationResult\(result, operationId, preview\)/);
  assert.match(view, /assertAppliedPreferences\(preferences, preview\)/);
  assert.match(view, /unresolvedApplication = \{ application: structuredClone\(value\)/);
  const release = view.slice(view.indexOf('const releaseUnknownApplication ='), view.indexOf('const setupContent ='));
  assert.match(release, /cancel_model_inspection.*token: value.operationId/);
  assert.match(release, /get_model_placements.*get_model_profiles/);
  assert.match(release, /entries.forEach\(placementLabel\); assertProfileCatalog\(views\)/);
  assert.match(release, /current.current.application\?\.operationId !== value.operationId/);
  assert.match(view, /保存の取消や巻き戻しは行いません/);
});

test("matching downloaded copy selects the installed model without inviting another copy", () => {
  const view = source("model-download.tsx"), parent = source("model-inspection.tsx");
  assert.match(view, /entry.placementRecorded && entry.sourceType === "download"/);
  assert.match(view, /entry.download\?\.operationId === receipt.operationId && entry.download.recipeHash === receipt.recipeHash && entry.reviewHash === receipt.report.reviewHash/);
  assert.match(view, /pending \|\| knownCopy \|\| !acknowledged/);
  assert.match(view, /if \(knownEntry\) \{ onPlaced\?\.\(knownEntry.packageId\)/);
  assert.match(parent, /onPlaced=\{setSelectedPackage\} setupContent=\{setupContent\}/);
});
