import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
test("minutes paper and basic toolbar use available width; opener shares the body's gutter", () => {
  const css=source("prototype-alignment.css"), base=source("style.css");
  assert.match(base,/\.minutes-split \.minutes-document,\.minutes-split \.minutes-wysiwyg \{ max-width:none;/);
  assert.match(css,/\.minutes-primary-toolbar \{[^}]*max-width:none;/);
  assert.match(css,/\.minutes-split \.minutes-document-layout \{ grid-template-columns:minmax\(0,1fr\); \}/);
  assert.match(css,/\.minutes-split\[data-full=false\]:has\([^}]*>\.workspace-panel-shell \{[^}]*padding:2px 8px 2px 2px; overflow:hidden; scrollbar-gutter:stable;/);
});
test("meeting title precedes tabs and all fixed chrome participates in dock sizing", () => {
  const main = source("main.tsx");
  assert.ok(main.indexOf('{detail && <div className="meeting-detail-status"') < main.indexOf('{detail && <div className="meeting-detail-navigation"'));
  assert.match(main, /heading\?\.getBoundingClientRect\(\)\.height/);
  assert.match(main, /querySelectorAll<HTMLElement>\("\.meeting-detail-status,\.meeting-detail-navigation"\)/);
});
test("meeting actions align with tabs in normal flow and wrapped navigation is measured", () => {
  const main=source("main.tsx"), css=source("prototype-alignment.css");
  assert.match(main,/className="meeting-detail-actions" ref=\{setMeetingActionsHost\}/);
  assert.match(main,/triggerHost=\{detail \? meetingActionsHost : null\}/);
  assert.match(main,/navigation\?\.getBoundingClientRect\(\)\.height/);
  assert.match(css,/\.meeting-detail-navigation \{[^}]*align-items:center; flex-wrap:wrap; gap:8px 16px;/);
  assert.match(css,/\.meeting-detail-navigation>\.meeting-detail-tabs \{ flex:1 1 162px;/);
  assert.match(css,/\.meeting-detail-actions \{ display:flex; align-items:center;/);
  assert.match(css,/\.meeting-workspace-tools \{ position:static; \}/);
  assert.doesNotMatch(css,/\.meeting-workspace-tools \{[^}]*position:absolute/);
});
test("transcript primary reading keeps guarded text and time actions while advanced inputs stay mounted", () => {
  const transcript = source("transcript.tsx");
  assert.match(transcript, /<span role="button" className="transcript-body-edit"[^\n]*aria-disabled=\{!editable && !canBeginText\} tabIndex=\{editable \|\| canBeginText \? 0 : -1\}/);
  assert.match(transcript, /shouldOpenTranscriptBody\(event.detail, body.ownerDocument.getSelection\(\), node => body.contains\(node\)\)\) requestTextEdit\(utterance\)/);
  assert.match(transcript, /if \(!editable && !canBeginText \|\| !shouldOpenTranscriptBodyFromKey\(event.nativeEvent\)\) return;\s*event.preventDefault\(\); requestTextEdit\(utterance\);/);
  assert.doesNotMatch(transcript, /<button[^>]*className="transcript-body-edit"/);
  assert.match(transcript, /className="utterance-stamp"[^>]*onClick=\{\(\) => void play\(utterance\)\}/);
  for (const section of ["speaker-merge-tools", "utterance-edit-tools"]) assert.match(transcript, new RegExp(`<OptionsModal className="${section}"`));
  assert.match(transcript, /<CommonModal ref=\{splitDialog\}/);
  assert.match(transcript, /<IssueNotice scope=\{meetingId\} type="transcript-save"/);
});

test("fine settings open modals rather than resizing the reading workspace", () => {
  const transcript=source("transcript.tsx"), minutes=source("minutes.tsx"), main=source("main.tsx");
  assert.match(transcript,/<OptionsModal className="workspace-tools transcript-workspace-tools"/);
  assert.match(transcript,/<div className="speaker-diarization-entry" ref=\{bindDiarizationHost\} hidden=\{!visible\}/);
  assert.match(source("diarization-panel.tsx"),/<OptionsModal className="speaker-diarization-tools" label="人数指定・話者分離" hidden=\{!visible\}/);
  assert.match(minutes,/<OptionsModal className="workspace-tools minutes-workspace-tools"/);
  assert.match(source("minutes-editor.tsx"),/<OptionsModal className="workspace-tools minutes-format-tools"/);
  assert.match(main,/<OptionsModal className=\{detail \? "meeting-workspace-tools workspace-tools"/);
  assert.match(main,/<section className="meetings-card" aria-label="保存済み会議" hidden=\{detail\}/);
  assert.doesNotMatch(transcript,/<details className="(?:workspace-tools|speaker-merge-tools|speaker-diarization-tools|utterance-edit-tools)/);
});

test("speaker names open a guarded modal without an extra rename row", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  assert.doesNotMatch(transcript,/<summary>名前を変更<\/summary>|className="speaker-name-tools"/);
  assert.match(transcript,/className="speaker-name-button"[^\n]*aria-haspopup="dialog"[^\n]*requestNameEdit\(speaker.id,event.currentTarget\)/);
  assert.match(transcript,/className="speaker-count-filter"[^\n]*setSpeakerFilter\(speaker.id\)/);
  assert.match(transcript,/className="speaker-name-editor speaker-name-dialog" aria-label="話者名を変更"/);
  assert.match(transcript,/await finishName\(id,kind\);[\s\S]*if \(nameBuffers.current\[id\]\) return;/);
  assert.match(transcript,/pending.kind === "name"/);
  assert.match(transcript,/if \(event.key === "Escape"\) event.stopPropagation\(\);/);
  assert.match(css,/\.speaker-list-row \{ display:flex;/);
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

test("transcript options flow with heading actions instead of reserving a fixed overlay width", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  assert.match(transcript, /className="transcript-heading-actions" ref=\{setOptionsHost\}/);
  assert.match(transcript, /triggerLabel="編集オプション" icon="edit" triggerHost=\{optionsHost\}/);
  assert.match(css, /\.transcript-heading-actions \{ display:flex; align-items:center; min-width:0; max-width:100%;/);
  assert.match(css, /\.transcript-main>\.transcript-actions:first-child \{ justify-content:flex-end; gap:8px 12px;/);
  assert.doesNotMatch(css, /\.transcript-workspace-tools \{[^}]*position:absolute/);
  assert.doesNotMatch(css, /padding-right:118px/);
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
  assert.match(css,/\.minutes-primary-toolbar \{ align-items:stretch; \}/);
  assert.match(css,/\.transcript-body-edit,\.utterance-play-icon,\.evidence-chip,\[data-evidence-id\]/);
});

test("transcript options group fields and spaced actions without individual scrollports", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  for (const title of ["表示と再読込", "保存と回復", "まとめて話者を変更"]) assert.ok(transcript.includes(`<h3>${title}</h3>`));
  assert.match(transcript,/className="workspace-tools-body transcript-options-body"/);
  assert.match(transcript,/className="transcript-options-bulk-controls"/);
  assert.match(transcript,/className="small-secondary" disabled=\{!selection.length\|\|busy\} onClick=\{\(\)=>setSelection\(\[\]\)\}>選択解除/);
  assert.match(css,/\.transcript-options-buttons,\.transcript-options-recovery \{[^}]*gap:10px;/);
  assert.match(css,/\.transcript-workspace-tools \.options-dialog \{ max-width:720px;/);
  assert.match(css,/\.transcript-options-section label \{ display:grid; gap:8px;/);
  assert.match(css,/@media\(max-width:600px\) \{\s*\.transcript-options-display,\.transcript-options-bulk-controls \{ grid-template-columns:minmax\(0,1fr\);/);
  assert.doesNotMatch(css,/\.transcript-options[^}]*\{[^}]*overflow:(?:auto|scroll)/);
});
test("minutes edit actions use the existing editor owner and generation stays guarded", () => {
  const panel = source("minutes.tsx"), editor = source("minutes-editor.tsx");
  assert.match(panel, /actionsHost=\{editorActionsHost\}/);
  assert.match(editor, /actionsHost \? createPortal\(primaryActions, actionsHost\) : primaryActions/);
  assert.match(editor, /await queue\.flush\(\);[\s\S]*setEditing\(false\)/);
  assert.match(editor, /focused === document\.activeElement/);
  assert.match(editor, /target\.focus\(\{ preventScroll: true \}\)/);
  assert.match(panel, /recordingActive \|\| pending \|\| !source\?\.currentId \|\| !engine \|\| !minutesConfigured\(engine\)/);
  assert.match(panel, /className="minutes-empty-state"/);
  const evidence = source("minutes-evidence.tsx");
  assert.match(evidence, /<InfoPopover label="根拠の確認"/);
  assert.match(evidence, /<details className="evidence-record-details"/);
  assert.match(evidence, /根拠の音声を再生/);
});

test("desktop transcript keeps the mock's left speaker column and direct editing guards", () => {
  const css=source("prototype-alignment.css"), transcript=source("transcript.tsx");
  assert.match(css,/\.meeting-detail-content \.transcript-split \{ grid-template-columns:216px minmax\(0,1fr\); grid-template-rows:minmax\(0,1fr\);/);
  assert.match(transcript,/<WorkspacePanel[^\n]*query="\(max-width: 760px\)"/);
  assert.match(css,/\.transcript-split \.workspace-drawer-trigger \{ min-height:28px; padding:4px 8px;/);
  assert.match(css,/\.meeting-detail-content \.transcript-split \{ gap:6px;/);
  assert.match(css,/@media\(max-height:760px\) \{\s*\.meeting-detail-status \{ padding-block:12px 8px; gap:0;/);
  assert.match(source("style.css"),/\.utterance-row textarea\{[^}]*resize:vertical;min-height:55px/);
  assert.match(transcript,/disabled=\{!editable\} rows=\{2\} maxLength=\{20000\}/);
  assert.match(transcript,/finishTextEdit\(utterance.id, shortcut === "cancel" \? "cancel" : "commit", event.currentTarget\)/);
  assert.match(transcript,/if \(!canBeginText \|\| pendingTextOpen.current \|\| !analysis\) return;/);
  assert.match(transcript,/if \(!editable \|\| !view\) return;/);
  assert.match(transcript,/document.activeElement !== pending.opener && document.activeElement !== document.body/);
});

test("transcript synthetic failure controls are opt-in and never invoke product saves", () => {
  const fixture=readFileSync(new URL("./detail-mock-browser-fixture.js",import.meta.url),"utf8").split('} else {')[1];
  assert.match(fixture,/get\("checks"\) === "1"/);
  assert.match(fixture,/window.monBrowserFixture.failSave = fail/);
  assert.match(fixture,/close.onclick = \(\) => \{ controls.hidden = true; \}/);
  assert.doesNotMatch(fixture,/\.invoke\(|\.setMeetingTitle\(|\.setUtterances\(/);
  assert.match(fixture,/payload:\{schemaVersion:1,theme:"system"\}/);
  assert.match(fixture,/c=>c.command===command/);
  assert.match(fixture,/update\(\);window.setInterval\(update,200\)/);
});

test("transcript feedback shares natural space without a nested scrollport and retains explicit retry", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  assert.match(transcript,/<div className="transcript-feedback" hidden=\{!error && !saveError\}>\s*\{error && <p className="action-error" role="alert">\{error\}<\/p>\}\s*<IssueNotice scope=\{meetingId\} type="transcript-save"/);
  assert.match(transcript,/run: \(\) => retryTextBuffers\(\)\.catch/);
  assert.match(css,/\.transcript-feedback \{ flex:none;[^}]*flex-wrap:wrap;/);
  assert.doesNotMatch(css,/\.transcript-feedback \{[^}]*overflow:/);
  assert.match(css,/\.transcript-feedback\[hidden\] \{ display:none;/);
  assert.match(css,/\.transcript-feedback>\.action-error \{[^}]*margin:0!important;/);
});

test("transcript loading and empty states retain responsive workspace insets", () => {
  const transcript=source("transcript.tsx"), css=source("prototype-alignment.css");
  assert.match(transcript,/loading && <p className="transcript-state" role="status"/);
  assert.match(transcript,/!loading && !analysis && !error && <p className="transcript-state"/);
  assert.match(css,/\.meeting-detail-content \.transcript-state \{ flex:none; padding:24px 32px;/);
  assert.match(css,/\.meeting-detail-content \.transcript-state \{ padding:20px 16px;/);
  assert.match(css,/\.transcript-feedback \{ padding-inline:16px;/);
});

test("analysis preview spaces metadata and keeps guarded accessible icon playback", () => {
  const analysis=source("analysis.tsx"), css=source("prototype-alignment.css");
  assert.match(analysis,/className="preview-utterance-meta"/);
  assert.match(analysis,/className="small-secondary utterance-play-icon" aria-label=\{`\$\{timeLabel\(item.startUs\)\}の発言を再生`\}/);
  assert.match(analysis,/disabled=\{locked\|\|audio.preparing\} onClick=\{\(\)=>void play\(item\)\}/);
  assert.match(analysis,/<svg[^>]*aria-hidden="true" focusable="false"/);
  assert.doesNotMatch(analysis,/>この発言を聞く<|data-dialog-cancel/);
  assert.match(analysis,/dismissLabel="閉じる"/);
  assert.match(css,/\.preview-utterance-meta \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px 12px;/);
  assert.match(css,/\.utterance-play-icon \{[^}]*width:32px; height:32px; min-height:32px; padding:0;/);
  assert.match(css,/\.transcript-body-edit,\.utterance-play-icon,\.evidence-chip/);
  assert.match(css,/\.common-modal\.analysis-dialog \.preview-columns \{ grid-template-columns:repeat\(2,minmax\(0,1fr\)\);/);
  assert.match(css,/\.common-modal\.analysis-dialog \.preview-columns \{ grid-template-columns:minmax\(0,1fr\);/);
});

test("mixed action rows and select fields align without removing wrapping or checkbox semantics", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.analysis-actions>span,[^{]*\{ align-self:center;/);
  assert.match(css,/\.card-actions \{ flex-wrap:wrap;/);
  assert.match(css,/\.meeting-form-row \{ align-items:center; flex-wrap:wrap;/);
  assert.match(css,/\.theme-choice select,\.common-modal select,\.capture-input-selection select \{ min-height:40px;[^}]*border-radius:8px;/);
  assert.match(css,/\.audio-controls label \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px;/);
  assert.match(css,/\.theme-choice select,\.common-modal select,\.audio-controls select,\.capture-input-selection select \{\s*-webkit-appearance:none; appearance:none; padding-right:32px;/);
  assert.match(css,/background-size:5px 5px; background-repeat:no-repeat;/);
  assert.match(css,/@media\(forced-colors:active\) \{\s*\.theme-choice select,\.common-modal select,\.audio-controls select,\.capture-input-selection select \{ -webkit-appearance:auto; appearance:auto; background-image:none;/);
  assert.match(css,/\.action-error:has\(>button\) \{ display:flex; align-items:center; flex-wrap:wrap; gap:8px 12px;/);
  assert.match(css,/\.vocabulary-list \.analysis-actions label \{ gap:8px;/);
  assert.match(css,/\.transcript-main>\.transcript-actions:first-child \{ min-height:32px;/);
  assert.match(css,/\.meeting-audio-settings-fields>\.playback-follow-choice \{ display:flex; align-items:center; gap:8px;/);
  assert.match(css,/\.import-term label:has\(>input\[type=checkbox\]\) \{ flex-direction:row; align-items:center; gap:8px;/);
});

test("minutes chrome keeps workspace insets and preview has one guarded dismiss action", () => {
  const css=source("prototype-alignment.css"), minutes=source("minutes.tsx");
  assert.match(css,/\.meeting-detail-content \.minutes-card \{ padding:24px 32px; gap:12px;/);
  assert.match(css,/\.meeting-detail-content \.minutes-card \{ padding:16px;/);
  assert.match(minutes,/dismissLabel: "閉じる", dismissText: "議事録プレビューを閉じる"/);
  assert.doesNotMatch(minutes,/data-dialog-cancel[^>]*>議事録プレビューを閉じる/);
});

test("secondary settings and vocabulary fields retain space between neighboring groups", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.settings-management-toggle \{ margin:18px 0;/);
  assert.match(css,/\.vocabulary-list article \{ display:grid; gap:12px;/);
  assert.match(css,/\.vocabulary-list label \{ min-width:0; gap:8px;/);
  assert.match(css,/\.vocabulary-list article>\.analysis-actions \{ margin:0; align-items:center;/);
  assert.match(css,/\.options-dialog-content>:is\(p,\.analysis-actions,\.prompt-label,\.analysis-settings\),[\s\S]*?\{ margin-block:0; \}/);
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

test("minutes format options space their nested explanation and wrapped actions explicitly", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.minutes-format-tools \.workspace-tools-body \{ display:grid; gap:16px; min-width:0; \}/);
  assert.match(css,/\.minutes-format-tools \.workspace-tools-body>p \{ margin:0; \}/);
  assert.match(css,/\.minutes-format-tools \.minutes-toolbar \{ margin:0; gap:10px; \}/);
});

test("minutes settings group portaled management with reload and generation without touching actions", () => {
  const css=source("prototype-alignment.css"), minutes=source("minutes.tsx");
  assert.match(minutes,/<div className="minutes-settings-controls">\s*<div ref=\{setEditorOptionsHost\}/);
  assert.match(css,/\.minutes-settings-controls \{ display:flex; flex-wrap:wrap; align-items:stretch; gap:10px; min-width:0; \}/);
  assert.match(css,/\.minutes-settings-controls>\.minutes-editor-options-host \{ display:contents; \}/);
  assert.match(css,/\.minutes-workspace-tools \.workspace-tools-body \{ display:grid; gap:20px; min-width:0; \}/);
  assert.match(css,/\.minutes-workspace-tools \.analysis-settings>fieldset \{ display:grid; gap:16px; min-width:0; margin:0; \}/);
});

test("pagination and metadata actions use the same metrics and can wrap", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.transcript-pages,\.utterance-meta,\.preview-utterance-meta,\.version-row,/);
  assert.match(css,/\.transcript-pages \{ flex-wrap:wrap; gap:10px 12px;/);
  assert.match(css,/\.transcript-pages :is\(form,label\) \{ min-width:0; flex-wrap:wrap; gap:8px;/);
  assert.match(css,/\.transcript-pages input\[type=number\] \{ min-height:var\(--action-height,40px\);/);
  assert.match(css,/\.version-row \{ flex-wrap:wrap; gap:12px;/);
  assert.match(css,/\.evidence-verification \{ flex-wrap:wrap; gap:8px 12px;/);
  assert.match(css,/\.utterance-meta \.assigned-speakers \{ margin:0; align-items:center;/);
});

test("reading surfaces reserve scrollbar clearance and short controls have no nested scrollport", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.meeting-detail-content \.utterance-list,\.meeting-detail-content \.minutes-body-scroll \{ padding-right:8px; scrollbar-gutter:stable;/);
  assert.match(css,/\.meeting-detail-content \.transcript-search \{ align-items:center; gap:8px 12px; max-height:none; overflow:visible;/);
  assert.match(css,/\.playback-follow-status \{ max-height:none; overflow:visible; padding-block:8px;/);
  assert.match(css,/\.info-popover-head \{ align-items:center;/);
  assert.match(css,/\.info-popover \.info-popover-head h3 \{ margin:0;/);
});

test("settings title margins cannot offset adjacent info controls and fields share action metrics", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.options-dialog \.heading-with-info>:is\(h2,h3\),\s*\.heading-with-info>label \{ margin:0; min-width:0; \}/);
  assert.match(css,/\.heading-with-info \{ gap:8px; \}/);
  assert.match(css,/\.common-modal input:not\(\[type=checkbox\]\):not\(\[type=radio\]\):not\(\[type=range\]\):not\(\[type=file\]\)[^{]*\{\s*min-width:0; max-width:100%; min-height:40px;\s*padding:9px 12px; font-size:13px; line-height:20px;/);
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

test("minutes reference headers are centered and expanded records cannot touch playback actions", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.minutes-reference-body \.fixed-evidence-group>\.card-head \{ align-items:center; gap:12px; \}/);
  assert.match(css,/\.fixed-transcript-workspace>\.card-head>h3 \{ min-width:0; margin:0; \}/);
  assert.match(css,/\.fixed-evidence>\.evidence-record-details \{ margin-block:12px; \}/);
  assert.match(css,/\.evidence-record-details>small \{ display:block; margin-top:8px; overflow-wrap:anywhere; \}/);
  assert.match(css,/\.fixed-evidence>\.audio-controls \{ align-items:stretch; gap:10px 12px; \}/);
  assert.match(css,/\.fixed-evidence>\.audio-controls>button \{ min-width:0; max-width:100%; \}/);
});

test("speaker count settings keep one spacing rhythm and full-width aligned fields", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.options-dialog \.diarization-panel \{ display:grid; gap:16px; min-width:0; margin:0; padding:0; border:0; \}/);
  assert.match(css,/\.options-dialog \.diarization-panel>\.heading-with-info>h3 \{ margin:0; \}/);
  assert.match(css,/\.options-dialog \.diarization-panel>\.analysis-settings \{ margin:0; \}/);
  assert.match(css,/\.options-dialog \.diarization-tracks \{[^}]*grid-template-columns:repeat\(2,minmax\(0,1fr\)\);[^}]*gap:16px;[^}]*margin:0;/);
  assert.match(css,/\.options-dialog \.diarization-tracks label \{[^}]*gap:8px; margin:0;/);
  assert.match(css,/\.options-dialog \.diarization-tracks :is\(input,select\) \{ width:100%; max-width:none; \}/);
  assert.match(css,/@media\(max-width:600px\) \{\s*\.options-dialog \.diarization-tracks \{ grid-template-columns:minmax\(0,1fr\); \}/);
});

test("recording and capture settings reuse field metrics without doubling label margins", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.recording-workspace input:not\(\[type=checkbox\]\):not\(\[type=range\]\),\s*\.capture-input-selection select \{\s*min-height:40px; padding:9px 12px; font-size:13px; line-height:20px;/);
  assert.match(css,/\.capture-scope-details \.options-dialog-content>label \{ display:grid; gap:8px; margin:0; min-width:0; \}/);
  assert.match(css,/\.recording-workspace-heading>button \{ flex:none; \}/);
  assert.match(css,/\.capture-input-selection select \{ padding-right:32px; \}/);
});

test("the narrow speaker drawer opener has insets instead of touching the tabs and workspace edge", () => {
  assert.match(source("prototype-alignment.css"), /@media\(max-width:760px\) \{\s*\.meeting-detail-content \.transcript-split>\.workspace-panel-shell \{ padding:12px 16px 0; \}/);
});

test("audio import uses spaced groups and the same field metrics as other settings", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.common-modal\.import-dialog \{ width:min\(720px,calc\(100vw - 32px\)\); max-width:720px;/);
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

test("import title and profile share a wide row while explanatory text and optional settings span it", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/@media\(min-width:600px\) \{\s*\.import-dialog fieldset \{ grid-template-columns:minmax\(0,2fr\) minmax\(0,1fr\); align-items:start; \}/);
  assert.match(css,/\.import-dialog fieldset>:is\(p,\.import-vocabulary-entry\) \{ grid-column:1 \/ -1; \}/);
  assert.match(css,/\.import-dialog \.import-field-label \{ display:flex; align-items:center; min-height:32px; \}/);
  assert.match(css,/\.import-dialog \.import-profile-field \{ display:grid; gap:8px; min-width:0; \}/);
  assert.match(css,/\.import-dialog\.import-vocabulary-view>\.common-modal-replacement:not\(\[hidden\]\) \{ display:grid; gap:16px; min-width:0; \}/);
  const panel=source("audio-import.tsx");
  assert.match(panel,/<label htmlFor="import-analysis-profile">解析プロファイル<\/label><InfoPopover label="取り込み後の解析"/);
  assert.match(panel,/<select id="import-analysis-profile" value=\{profile\}/);
  assert.match(panel,/>会議の用語（任意）<\/button><InfoPopover label="取り込みの用語"/);
  assert.doesNotMatch(panel,/<h3>取り込み後の解析<\/h3>|<h3>会議の用語<\/h3>/);
  assert.doesNotMatch(css,/\.import-dialog fieldset[^}]*display:none/);
});

test("pointer opening an import captures its actual entry point before the native file panel", () => {
  const panel=source("audio-import.tsx");
  assert.match(panel,/onClick=\{event=>\{event.currentTarget.focus\(\{preventScroll:true\}\);void select\(null\);\}\}>音声を取り込む/);
  assert.match(panel,/const sourceOpener = document.activeElement instanceof HTMLElement \? document.activeElement : null;/);
  assert.match(panel,/if \(!open\) opener.current=sourceOpener;/);
  assert.match(panel,/restoreTarget=\{\(\) => opener.current\}/);
});

test("maintenance headings and immediate actions have spacing independent of optional text", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.settings-management-toggle \.options-dialog-content>\.analysis-card \{\s*display:grid; gap:16px; min-width:0; margin:0;/);
  assert.match(css,/\.settings-management-toggle \.options-dialog-content>\.analysis-card>:is\(h2,p,ul,\.analysis-actions\) \{ margin-block:0;/);
  assert.match(css,/\.settings-management-toggle \.options-dialog-content>\.analysis-card>button \{ justify-self:start; max-width:100%; \}/);
  assert.match(css,/\.settings-management-toggle \.options-dialog-content>\.analysis-card>ul:empty \{ display:none; \}/);
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

test("minutes page controls use explicit group spacing and matching field insets", () => {
  assert.match(source("minutes-pages.tsx"),/<nav className="minutes-page-navigation"/);
  const css=source("prototype-alignment.css");
  assert.match(css,/\.minutes-page-navigation \{ display:grid; gap:12px; min-width:0; margin-top:16px; \}/);
  assert.match(css,/\.minutes-page-navigation>\.transcript-pages,\.minutes-page-navigation>\.action-error \{ margin:0; \}/);
  assert.match(css,/\.minutes-page-navigation \.transcript-pages input\[type=number\],[\s\S]*?padding:calc\(\(var\(--action-height,40px\) - var\(--action-line-height,20px\) - 2px\) \/ 2\) 12px;/);
});

test("modal row headings cannot use default margins to offset neighboring controls", () => {
  assert.match(source("prototype-alignment.css"),/\.common-modal \.card-head>:is\(h2,h3,h4\) \{ min-width:0; margin:0; \}/);
});

test("import spacing cannot reveal a body hidden behind its owned confirmation", () => {
  const css=source("prototype-alignment.css");
  assert.doesNotMatch(css,/\.import-dialog>\.common-modal-body\s*\{[^}]*display:/);
  assert.match(source("style.css"),/\.common-modal-body\[hidden\] \{ display:none; \}/);
});
