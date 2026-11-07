import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
test("meeting title precedes tabs and all fixed chrome participates in dock sizing", () => {
  const main = source("main.tsx");
  assert.ok(main.indexOf('{detail && <div className="meeting-detail-status"') < main.indexOf('{detail && <div className="meeting-detail-navigation"'));
  assert.match(main, /heading\?\.getBoundingClientRect\(\)\.height/);
  assert.match(main, /querySelectorAll<HTMLElement>\("\.meeting-detail-status,\.meeting-detail-navigation"\)/);
});
test("meeting actions align with tabs in normal flow and wrapped navigation is measured", () => {
  const main=source("main.tsx"), css=source("prototype-alignment.css");
  assert.match(main,/className="meeting-detail-actions"><RowMenu/);
  assert.match(main,/navigation\?\.getBoundingClientRect\(\)\.height/);
  assert.match(css,/\.meeting-detail-navigation \{[^}]*align-items:center; flex-wrap:wrap; gap:8px 16px;/);
  assert.match(css,/\.meeting-detail-navigation>\.meeting-detail-tabs \{ flex:1 1 162px;/);
  assert.match(css,/\.meeting-detail-actions \{ display:flex; align-items:center;/);
  assert.match(css,/\.meeting-workspace-tools \{ position:static; \}/);
  assert.doesNotMatch(css,/\.meeting-workspace-tools \{[^}]*position:absolute/);
});



test("speaker labels keep only the mock avatar background in dark mode", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.speaker-panel-body \.speaker-label,\.utterance-meta \.speaker-label \{[^}]*padding:0;[^}]*min-width:0; flex-wrap:nowrap;/);
  assert.match(css,/:root\[data-theme=dark\] \.speaker-panel-body \.speaker-label,:root\[data-theme=dark\] \.utterance-meta \.speaker-label \{ background:transparent; color:var\(--text\);/);
  assert.match(css,/\.speaker-label>span:not\(\.speaker-avatar\) \{ min-width:0; overflow-wrap:anywhere;/);
});

test("speaker heading leaves explicit space before the all-utterances filter", () => {
  assert.match(source("prototype-alignment.css"), /\.speaker-heading \{[^}]*margin-bottom:16px;/);
});


test("utterance menus participate in the metadata grid rather than floating against the edge", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.utterance-row \{[^}]*display:grid; grid-template-columns:minmax\(0,1fr\) auto; column-gap:12px;/);
  assert.match(css,/\.utterance-row>\.utterance-meta \{[^}]*min-height:32px;/);
  assert.match(css,/\.utterance-edit-tools\.options-modal \{ position:static; grid-column:2; grid-row:1;/);
  assert.match(css,/\.utterance-row>:not\(\.utterance-meta\):not\(\.utterance-edit-tools\) \{ grid-column:1 \/ -1;/);
  assert.doesNotMatch(css,/\.utterance-edit-tools\.options-modal \{[^}]*position:absolute/);
  assert.match(css,/min-width:0; max-width:100%; overflow-wrap:anywhere;\s*vertical-align:middle; white-space:normal;/);
});

test("narrow playback places its description on its own row instead of squeezing it behind actions", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.meeting-playback-dock>\.meeting-playback-content \{ flex-basis:100%; \}/);
  assert.match(css,/\.meeting-playback-dock \.audio-controls \{ flex-wrap:wrap; \}/);
  assert.match(css,/\.meeting-playback-dock \.meeting-audio-actions \{ flex-wrap:wrap; max-width:100%; \}/);
});

test("settings dialogs use the declared width instead of inheriting the smaller confirmation cap", () => {
  assert.match(source("prototype-alignment.css"), /\.options-dialog \{ width:min\(720px,calc\(100vw - 32px\)\); max-width:720px;/);
});

test("toast text and icon controls share the vertical center instead of top alignment", () => {
  const css=source("style.css");
  assert.match(css,/\.toast-card \{ display:flex; align-items:center;/);
  assert.doesNotMatch(css,/\.toast-card \{[^}]*align-items:flex-start/);
  for (const selector of ["toast-icon", "toast-close"]) assert.match(css,new RegExp(`\\.${selector} \\{ display:inline-flex; align-items:center; justify-content:center;[^}]*line-height:1;`));
});

test("adjacent text actions share height and typography across portaled triggers and modal rows", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/min-height:var\(--action-height,40px\)/);
  assert.match(css,/font-family:inherit; font-size:var\(--action-font-size,13px\)/);
  assert.match(css,/line-height:var\(--action-line-height,20px\); font-weight:500/);
  assert.match(css,/--action-height:32px; --action-font-size:12px; --action-line-height:18px/);
  assert.match(css,/\.common-modal,\.common-modal :is\([\s\S]*?--action-height:40px; --action-font-size:13px; --action-line-height:20px;/);
  assert.match(source("minutes-markdown-editor.css"),/\.plain-minutes-actions \{ display:flex; align-items:center;/);
  assert.match(css,/\.transcript-body-edit,\.utterance-play-icon/);
});

test("minutes edit actions use the existing editor owner and generation stays guarded", () => {
  const panel=source("minutes.tsx"), editor=source("minutes-markdown-editor.tsx");
  assert.match(panel,/actionsHost=\{actionsHost\}/);
  assert.match(editor,/actionsHost \? createPortal\(actions, actionsHost\) : actions/);
  assert.match(panel,/recordingActive \|\| pending \|\| preparing \|\| !source\?\.currentId \|\| !engine \|\| !minutesConfigured\(engine\)/);
  assert.match(panel,/start_markdown_minutes_generation/);
  assert.doesNotMatch(panel,/start_minutes_generation"|adopt_minutes_candidate|restore_minutes_version/);
});


test("transcript synthetic failure controls are opt-in and never invoke product saves", () => {
  const fixture=readFileSync(new URL("./detail-mock-browser-fixture.js",import.meta.url),"utf8").split('} else {')[1];
  assert.match(fixture,/get\("checks"\) === "1"/);
  assert.match(fixture,/window.monBrowserFixture.failSave = fail/);
  assert.match(fixture,/close.onclick = \(\) => \{ controls.hidden = true; \}/);
  assert.doesNotMatch(fixture,/\.invoke\(|\.setMeetingTitle\(|\.setUtterances\(/);
  assert.match(fixture,/payload:\{schemaVersion:1,theme:"system", colorTheme:"monochrome"\}/);
  assert.match(fixture,/c=>c.command===command/);
  assert.match(fixture,/update\(\);window.setInterval\(update,200\)/);
});


test("transcript loading and empty states retain responsive workspace insets", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  assert.match(transcript,/loading && <p className="transcript-state" role="status"/);
  assert.match(transcript,/!loading && !analysis && !error && <p className="transcript-state"/);
  assert.match(css,/\.content\.meeting-detail-content \{ --tab-header-padding-top:24px; --tab-content-padding-inline:32px;/);
  assert.match(css,/@media\(max-width:760px\) \{\s*\.content\.meeting-detail-content \{ --tab-header-padding-top:12px; --tab-content-padding-inline:16px;/);
  assert.match(css,/\.meeting-detail-content \.transcript-state \{ flex:none; padding:24px var\(--tab-content-padding-inline\);/);
  assert.match(css,/\.meeting-detail-content \.transcript-state \{ padding-block:20px;/);
  assert.match(css,/\.transcript-feedback \{ padding-inline:16px;/);
});


test("mixed action rows and select fields align without removing wrapping or checkbox semantics", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.analysis-actions>span,[^{]*\{ align-self:center;/);
  assert.match(css,/\.card-actions \{ flex-wrap:wrap;/);
  assert.match(css,/\.meeting-form-row \{ align-items:center; flex-wrap:wrap;/);
  assert.match(css,/:root select \{ min-height:40px;[^}]*border-radius:8px;/);
  assert.match(css,/\.audio-controls label \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px;/);
  assert.match(css,/:root select:where\(:not\(\[multiple\]\)\) \{\s*-webkit-appearance:none; appearance:none; padding-right:32px;/);
  assert.match(css,/background-size:5px 5px; background-repeat:no-repeat;/);
  assert.match(css,/@media\(forced-colors:active\) \{\s*:root select:where\(:not\(\[multiple\]\)\) \{ -webkit-appearance:auto; appearance:auto; background-image:none;/);
  assert.match(css,/\.action-error:has\(>button\) \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px 12px;/);
  assert.match(css,/\.vocabulary-list \.analysis-actions label \{ gap:8px;/);
  assert.match(css,/\.transcript-main>\.transcript-actions:first-child \{ min-height:32px;/);
  assert.doesNotMatch(css,/playback-follow-choice|text-size-choice/);
  assert.match(css,/\.import-term label:has\(>input\[type=checkbox\]\) \{ flex-direction:row; align-items:center; gap:8px;/);
});

test("minutes chrome keeps workspace insets and preview has one guarded dismiss action", () => {
  const css=source("prototype-alignment.css"), minutes=source("minutes.tsx");
  assert.match(css,/\.meeting-detail-content \.minutes-card \{ padding:var\(--tab-header-padding-top\) var\(--tab-content-padding-inline\) 24px; gap:12px;/);
  assert.match(css,/\.meeting-detail-content \.minutes-card \{ padding-bottom:12px; gap:8px;/);
  assert.match(minutes,/<MinutesMarkdownEditor/);
  assert.doesNotMatch(minutes,/reviewContent|openPreview|議事録候補/);
});

test("secondary settings and vocabulary fields retain space between neighboring groups", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.vocabulary-list article \{ display:grid; gap:12px;/);
  assert.match(css,/\.vocabulary-list label \{ min-width:0; gap:8px;/);
  assert.match(css,/\.vocabulary-list article>\.analysis-actions \{ margin:0; align-items:center;/);
  assert.match(css,/\.options-dialog-content>:is\(p,\.analysis-actions,\.prompt-label,\.analysis-settings\) \{ margin-block:0; \}/);
  assert.match(css,/input\[type=file\]::file-selector-button \{ margin-right:12px;/);
  assert.match(css,/\.model-download-settings label:has\(>input\[type=file\]\) \{ display:grid; gap:8px; margin-block:0;/);
  assert.match(css,/\.model-download \[aria-label="未確定のモデル取得操作"\] \{ display:flex; align-items:center; flex-wrap:wrap; gap:10px;/);
  assert.match(css,/\.import-dialog nav\[aria-label="取り込み用語のページ"\] \{ display:flex; align-items:center; flex-wrap:wrap; gap:10px 12px;/);
});

test("vocabulary fields match adjacent actions without resizing checkboxes or multiline notes", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.vocabulary-list label \{[^}]*font-size:13px; line-height:20px;/);
  assert.match(css,/\.vocabulary-list :is\(input:not\(\[type=checkbox\]\),textarea\) \{[^}]*padding:9px 12px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.vocabulary-list input:not\(\[type=checkbox\]\) \{ min-height:40px; \}/);
  assert.doesNotMatch(css,/\.vocabulary-list input\[type=checkbox\] \{[^}]*min-height:40px/);
});

test("workspace search fields override inherited input fonts and retain consistent insets", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/:root \.meeting-search \{[^}]*font-size:13px; line-height:20px;/);
  assert.match(css,/\.meeting-detail-content \.transcript-search input\[type=search\] \{[^}]*min-height:40px; padding:9px 12px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.meetings-card>\.card-head>button \{[^}]*min-height:40px;[^}]*font-size:13px; line-height:20px;/);
});


test("minutes actions remain direct without an operations modal", () => {
  const css=source("minutes-markdown-editor.css"), minutes=source("minutes.tsx");
  assert.doesNotMatch(minutes,/OptionsModal|plain-minutes-option-actions|plain-minutes-options-body/);
  assert.match(minutes,/className="plain-minutes-primary-actions"/);
  assert.match(css,/\.confirmation-actions \{ display:flex; align-items:center; flex-wrap:wrap; gap:12px;/);
  assert.doesNotMatch(css,/plain-minutes-option-actions|plain-minutes-options-body/);
  assert.doesNotMatch(minutes,/save_minutes_preferences|engine-numbers|executablePath.*<input/);
});

test("pagination and metadata actions use the same metrics and can wrap", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.transcript-pages,\.utterance-meta,\.preview-utterance-meta,\.version-row,/);
  assert.match(css,/\.transcript-pages \{ flex-wrap:wrap; gap:10px 12px;/);
  assert.match(css,/\.transcript-pages :is\(form,label\) \{ min-width:0; flex-wrap:wrap; gap:8px;/);
  assert.match(css,/\.transcript-pages input\[type=number\] \{ min-height:var\(--action-height,40px\);/);
  assert.match(css,/\.version-row \{ flex-wrap:wrap; gap:12px;/);
  assert.match(css,/\.utterance-meta \.assigned-speakers \{ margin:0; align-items:center;/);
});

test("reading surfaces reserve scrollbar clearance and short controls have no nested scrollport", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.meeting-detail-content \.utterance-list \{ padding-right:8px; scrollbar-gutter:stable;/);
  assert.match(css,/\.meeting-detail-content \.transcript-search \{ align-items:center; gap:8px 12px; max-height:none; overflow:visible;/);
  assert.doesNotMatch(css,/playback-follow-status/);
  assert.match(css,/\.info-popover-head \{ align-items:center;/);
  assert.match(css,/\.info-popover \.info-popover-head h3 \{ margin:0;/);
});

test("settings title margins cannot offset adjacent info controls and fields share action metrics", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.options-dialog \.heading-with-info>:is\(h2,h3\),\s*\.heading-with-info>label \{ margin:0; min-width:0; \}/);
  assert.match(css,/\.heading-with-info \{ gap:8px; \}/);
  assert.match(css,/\.common-modal input:not\(\[type=checkbox\]\):not\(\[type=radio\]\):not\(\[type=range\]\):not\(\[type=file\]\)[^{]*\{\s*box-sizing:border-box; min-width:0; max-width:100%; min-height:40px;\s*padding:9px 12px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.options-dialog-content>fieldset \{\s*display:grid; gap:16px; min-width:0; margin:0; padding:0; border:0;/);
  assert.match(css,/\.options-dialog-content>fieldset>label \{\s*min-width:0; margin:0; gap:8px;/);
  assert.match(css,/\.options-dialog-content>\.info-field \{ display:grid; gap:8px; margin:0; \}/);
  assert.match(css,/\.engine-numbers \{ align-items:end; gap:12px; \}/);
});

test("nested model settings reserve space between headings, fields, records and actions", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.options-dialog-content>form \{\s*display:grid; gap:16px; min-width:0;/);
  assert.match(css,/\.options-dialog-content>form>:is\(\.prompt-label,\.analysis-actions,p\) \{ margin-block:0; \}/);
  assert.match(css,/\.model-inspection-settings \.options-dialog-content>section \{\s*display:grid; gap:12px; min-width:0;/);
  assert.match(css,/\.model-inspection-settings \.options-dialog-content>section>button \{ justify-self:start; max-width:100%; \}/);
  assert.match(css,/\.model-inspection-settings \.options-dialog-content>section>article \{\s*display:grid; gap:8px;/);
});


test("recording and capture settings reuse field metrics without doubling label margins", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.recording-workspace input:not\(\[type=checkbox\]\):not\(\[type=range\]\),\s*\.capture-input-selection select \{\s*min-height:40px; padding:9px 12px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.capture-scope-details \.options-dialog-content>label \{ display:grid; gap:8px; margin:0; min-width:0; \}/);
  assert.match(css,/\.recording-workspace-heading>button \{ flex:none; \}/);
  assert.match(css,/\.capture-input-selection select \{ padding-right:32px; \}/);
});

test("the narrow speaker drawer opener has insets instead of touching the tabs and workspace edge", () => {
  assert.match(source("prototype-alignment.css"), /@media\(max-width:760px\) \{\s*\.content\.meeting-detail-content \{[^}]*--tab-content-padding-inline:16px;[^}]*\}\s*\.meeting-detail-content \.transcript-split>\.workspace-panel-shell \{ padding:12px var\(--tab-content-padding-inline\) 0; \}/);
});

test("audio import uses spaced groups and the same field metrics as other settings", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.common-modal\.import-dialog \{ width:min\(720px,calc\(100% - 48px\)\); max-width:720px;/);
  assert.match(css,/\.import-dialog>\.common-modal-body:not\(\[hidden\]\) \{ display:grid; gap:16px; min-width:0;/);
  assert.match(css,/\.import-dialog>\.common-modal-body>:is\(h2,p,dl,fieldset,\.analysis-actions\) \{ margin-block:0;/);
  assert.match(css,/\.import-dialog \.heading-with-info>h3 \{ margin:0; font-size:14px;/);
  assert.match(css,/\.import-dialog fieldset \{ display:grid; gap:16px; margin:0;/);
  assert.match(css,/\.import-dialog fieldset>label \{ margin:0; gap:8px; font-size:13px;/);
  assert.match(css,/\.import-dialog fieldset>label>:is\(input,select\) \{ min-height:40px; padding-block:9px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.import-dialog \.analysis-actions \{ margin:0; padding:16px 0 0;/);
});

test("import metadata pairs use compact baseline-aligned columns and readable narrow groups", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.common-modal\.import-dialog \.import-info \{ grid-template-columns:116px minmax\(0,1fr\); align-items:baseline; gap:6px 16px;/);
  assert.match(css,/\.common-modal\.import-dialog \.import-info :is\(dt,dd\) \{ min-width:0; margin:0; \}/);
  assert.match(css,/@media\(max-width:480px\) \{\s*\.common-modal\.import-dialog \.import-info \{ grid-template-columns:minmax\(0,1fr\); row-gap:4px; \}/);
  assert.match(css,/\.common-modal\.import-dialog \.import-info dd:not\(:last-child\) \{ margin-bottom:8px; \}/);
  const panel=source("audio-import.tsx");
  for (const label of ["ファイル名","長さ","形式","原本","保存に必要な容量","保存先の空き容量"]) assert.ok(panel.includes(`<dt>${label}</dt>`));
});

test("import title uses the available width without a meeting model selector", () => {
  const css=source("prototype-alignment.css"), panel=source("audio-import.tsx");
  assert.match(css,/\.import-dialog fieldset \{ grid-template-columns:minmax\(0,1fr\); \}/);
  assert.doesNotMatch(panel,/import-analysis-profile|import-profile-field|options:.*profile/);
  assert.match(panel,/>会議の用語（任意）<\/button><VocabularyHelp label="取り込みの用語"/);
});

test("pointer opening an import captures its actual entry point before the native file panel", () => {
  const panel=source("audio-import.tsx");
  assert.match(panel,/onClick=\{event=>\{event.currentTarget.focus\(\{preventScroll:true\}\);void select\(null\);\}\}>音声を取り込む/);
  assert.match(panel,/const sourceOpener = document.activeElement instanceof HTMLElement \? document.activeElement : null;/);
  assert.match(panel,/if \(!open\) opener.current=sourceOpener;/);
  assert.match(panel,/restoreTarget=\{\(\) => opener.current\}/);
});


test("model acquisition records and consent actions keep natural gaps and one scroll owner", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.model-download-settings \.model-download-catalog \{[^}]*gap:16px;[^}]*max-height:none; overflow:visible;/);
  assert.match(css,/\.model-download-catalog>li>p \{ margin-block:8px 12px; \}/);
  assert.match(css,/\.model-download \.model-application-dialog>\.common-modal-body:not\(\[hidden\]\) \{ display:grid; gap:16px; min-width:0; \}/);
  assert.match(css,/\.model-download \.model-application-dialog>\.common-modal-body>button \{ justify-self:start; max-width:100%; \}/);
});

test("playback settings use one group gap instead of stacking label margins", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.meeting-audio-settings-fields>label \{ min-width:0; margin:0; gap:8px; \}/);
});

test("restored meeting notices separate the message and action without inline whitespace hacks", () => {
  assert.match(source("main.tsx"),/restoredFilter && <p className="meeting-list-notice">/);
  assert.match(source("prototype-alignment.css"),/\.meeting-list-notice \{ display:flex; align-items:center; flex-wrap:wrap; gap:10px 12px;/);
});


test("modal row headings cannot use default margins to offset neighboring controls", () => {
  assert.match(source("prototype-alignment.css"),/\.common-modal \.card-head>:is\(h2,h3,h4\) \{ min-width:0; margin:0; \}/);
});

test("import spacing cannot reveal a body hidden behind its owned confirmation", () => {
  const css=source("prototype-alignment.css");
  assert.doesNotMatch(css,/\.import-dialog>\.common-modal-body\s*\{[^}]*display:/);
  assert.match(source("style.css"),/\.common-modal-body\[hidden\] \{ display:none; \}/);
});
