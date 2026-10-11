import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
test("settings keep mounted modal inputs and attention never opens a background modal", () => {
  const view = source("settings-disclosure.tsx");
  assert.match(view,/<OptionsModal label=\{label\} className=\{`settings-disclosure \$\{className\}`\} attention=\{attention\}[^\n]*>\{children\}/);
  assert.doesNotMatch(view,/useEffect|setOpen|disabled=\{attention\}/);
  const modal=source("options-modal.tsx");
  assert.match(modal,/aria-haspopup="dialog"/);
  assert.match(modal,/<CommonModal[^\n]*open=\{open && !hidden\} nested data-settings-modal/);
  assert.match(modal,/\{children\}<\/div>/);
  assert.doesNotMatch(modal,/open && children|flush\(|invoke\(/);
});

test("model settings keep their spacing scope on the portaled dialog host", () => {
  const view=source("settings-disclosure.tsx"), modal=source("options-modal.tsx"), css=source("prototype-alignment.css");
  assert.match(view,/className\?: string/);
  assert.match(modal,/createPortal\(<div className=\{`options-modal \$\{className\}`\}/);
  assert.equal((source("model-inspection.tsx").match(/<SettingsDisclosure[^\n]*className="model-inspection-settings"/g) ?? []).length,2);
  assert.equal((source("model-download.tsx").match(/<SettingsDisclosure[^\n]*className="model-download-settings"/g) ?? []).length,3);
  assert.match(source("model-download.tsx"), /label="モデルをダウンロード" className="model-download-settings"/);
  assert.match(css,/\.model-inspection-settings \.options-dialog-content>section \{\s*display:grid; gap:12px;/);
  assert.match(css,/\.model-download-settings \.model-download-catalog \{[^}]*max-height:none; overflow:visible;/);
  assert.match(css,/\.model-inspection-settings \.prompt-label input \{[^}]*border:1px solid/);
  assert.match(css,/\.model-inspection-settings pre \{[^}]*white-space:pre-wrap; overflow-wrap:anywhere;/);
  assert.doesNotMatch(css,/\.model-(?:inspection|download) \.options-dialog-content/);
});
test("normal settings keep primary theme and model summary but fold diagnostics", () => {
  const main = source("main.tsx"), theme = source("theme.tsx"), models = source("model-inspection.tsx");
  assert.match(main, /<ThemeSettingsPanel compact locked/);
  assert.match(main, /<ModelInspectionPanel compact locked/);
  assert.ok(theme.indexOf('<ThemeSelection locked={locked} />') < theme.indexOf('<SettingsDisclosure label="本文と再生の表示設定"'));
  assert.match(theme, /attention=\{!!\(state.error \|\| state.busy \|\| state.uncertain \|\| state.pendingPreferences\)\}/);
  assert.match(models, /standard \? profileConfigurationLabel\(standard\)/);
  assert.match(models, /attention=\{recordsAttention\}/);
  assert.match(models, /entry.bodyRemoved !== true && entry.compatibility\?\.state !== "succeeded"/);
  assert.match(models, /<\/SettingsDisclosure>\s*\{deletionReview && <CommonModal/);
});
test("download recovery and confirmation are not hidden with the form or catalog", () => {
  const view = source("model-download.tsx");
  assert.match(view, /<\/SettingsDisclosure>\s*\{!!busy && <p role="status"/);
  assert.match(view, /<\/SettingsDisclosure>\s*<CommonModal open=\{!!modal\}/);
  assert.match(view, /entry.state !== "completion_recorded"/);
  assert.match(view, /<SettingsDisclosure label="取得定義からダウンロード"[^\n]*pending \|\| modal \|\| error/);
  const form = view.slice(view.indexOf('<SettingsDisclosure label="取得定義からダウンロード"'), view.indexOf('<SettingsDisclosure label="取得記録"'));
  assert.doesNotMatch(form.split('>')[0], /\|\| review|\|\| receipt/);
  assert.ok(form.indexOf('aria-label="照合済み取得素材"') > form.indexOf('</SettingsDisclosure>'));
});

test("model preflight results stay inside the initiating modal and next-step navigation never executes work", () => {
  const view=source("model-inspection.tsx");
  const manual=view.slice(view.indexOf('<SettingsDisclosure label="ローカルから取り込む"'),view.indexOf('<ModelDownloadPanel'));
  assert.match(manual,/aria-label="モデル検査結果"/);
  assert.match(manual,/onClick=\{\(\) => void place\(\)\}>管理領域へコピー/);
  assert.match(manual,/onClick=\{\(\) => setPreparationView\("records"\)\}>配置・構成記録を開く/);
  assert.match(manual,/disabled=\{busy\} onClick/);
  assert.match(manual,/setDirectory\(event.target.value\); setReport\(null\); setError\(""\); setNotice\(null\)/);
  assert.doesNotMatch(view.slice(view.indexOf("const inspect ="),view.indexOf("const stoppedApplication")),/setPreparationView/);
  assert.match(view,/preparationView === view && !confirm.pending && !\(application && applicationVisible\)/);
  assert.match(view,/<IssueStatus label="モデル準備" message=\{error\}/);
  for (const target of ["manual","records"]) {
    assert.ok(view.includes(`open={preparationView === "${target}"}`));
    assert.ok(view.includes(`restoreTarget={() => ${target}Trigger.current}`));
    assert.ok(view.includes(`{preparationFeedback("${target}")}`));
  }
});

test("controlled settings pass presentation and an explicit restore target without save or polling effects", () => {
  const view=source("settings-disclosure.tsx"), modal=source("options-modal.tsx");
  assert.match(view,/open=\{open\} onOpenChange=\{onOpenChange\} triggerRef=\{triggerRef\} restoreTarget=\{restoreTarget\}/);
  assert.match(modal,/<CommonModal[^\n]*restoreTarget=\{restoreTarget\}/);
  assert.doesNotMatch(view,/useEffect|invoke\(|\.flush\(/);
});

test("download record readback exposes the next step only after completion, with no automatic copy or execution", () => {
  const view = source("model-download.tsx");
  const inspect = view.slice(view.indexOf("const inspectEntry ="), view.indexOf("const supported ="));
  assert.match(inspect, /result.state === "complete" && recordsVisible.current/);
  assert.match(inspect, /recordsCopyFocus.current = true; setRecordsOpen\(false\)/);
  assert.match(inspect, /setRecordsFeedback\(feedback\)/);
  assert.match(inspect, /setRecordsError\(String\(cause\)\)/);
  assert.doesNotMatch(inspect, /start_model_download|place_downloaded_model|run_model|apply_model|copy\(\)|start\(\)/);
  const records = view.slice(view.indexOf('<SettingsDisclosure label="取得記録"'), view.indexOf('<CommonModal open={!!modal}'));
  assert.match(records, /restoreTarget=\{restoreRecordsFocus\}/);
  assert.match(records, /recordsCopyFocus.current = false; setRecordsOpen\(open\)/);
  assert.match(records, /<IssueStatus label="取得素材の照合" message=\{recordsError\}/);
  assert.match(records, /aria-label="取得記録の照合結果"/);
  assert.match(view, /<button ref=\{receiptButton\}/);
});

test("completed download readback reveals its restored action without scrolling ordinary cancellation or stealing focus", () => {
  const view = source("model-download.tsx");
  const restore = view.slice(view.indexOf("const restoreRecordsFocus ="), view.indexOf("const supported ="));
  assert.match(restore, /recordsCopyFocus.current \? receiptButton.current : recordsTrigger.current/);
  assert.match(restore, /if \(recordsCopyFocus.current && target\) queueMicrotask/);
  assert.match(restore, /recordsCopyFocus.current && !recordsVisible.current && document.activeElement === target/);
  assert.match(restore, /target.scrollIntoView\(\{ block: "nearest", inline: "nearest" \}\)/);
  assert.doesNotMatch(restore, /\.focus\(|invoke\(|start\(|copy\(/);
});

test("model help describes guided preparation and preserves formal quality and execution boundaries", () => {
  const view = source("model-inspection.tsx");
  assert.match(view, /「モデルをダウンロード」から文字起こし・話者分離・議事録生成を選べます/);
  assert.match(view, /管理コピー・CPU動作確認・標準／高精度への適用を別々に確認/);
  assert.match(view, /正式品質・性能とWindowsの取得は保留/);
  assert.match(view, /内包CLIの正式署名・公証は未検証/);
  assert.match(view, /保存した構成は新規解析が参照し、既存ジョブの条件は変更しません/);
  assert.doesNotMatch(view, /配布先・実モデルの検証とWindowsの取得は保留|管理パッケージとジョブの構成版参照への統合は開発中/);
});

test("download readback distinguishes material inspection from a matching copy record and opens records without recopy", () => {
  const view = source("model-download.tsx"), parent = source("model-inspection.tsx");
  assert.match(view, /entry.placementRecorded && entry.sourceType === "download"/);
  assert.match(view, /entry.download\?\.operationId === receipt.operationId && entry.download.recipeHash === receipt.recipeHash && entry.reviewHash === receipt.report.reviewHash/);
  assert.match(view, /pending \|\| knownCopy \|\| !acknowledged/);
  assert.match(view, /if \(knownCopy\) \{ onShowPlacements\?\.\(\); return; \}/);
  assert.match(view, /knownCopy \? "配置・構成記録を開く" : "照合済み素材のコピーを確認"/);
  assert.match(parent, /placements=\{placements\} onShowPlacements=\{\(\) => setPreparationView\("records"\)\}/);
  assert.doesNotMatch(view, /素材照合済み・未配置|選択した素材を照合しました。未配置・未実行/);
});
