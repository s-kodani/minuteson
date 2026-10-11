// Existing synthetic IPC fixtures only. No native windows, audio or file writes.
if (new URLSearchParams(location.search).get("view") === "minutes") {
  const { installMinutesFixture } = await import("./minutes-browser-fixture.js");
  const { minutesFixtureIntegrity } = await import("./minutes-browser-helpers.js");
  await installMinutesFixture({ addInitScript: async install => install(), reload: async () => {} });
  const parameters = new URLSearchParams(location.search);
  if (parameters.get("long") === "1") window.__mon.addLongDocument();
  if (parameters.get("group") === "1") window.__mon.addEvidenceGroup();
  if (parameters.get("outline") === "1") window.__mon.addOutlineDocument();
  if (parameters.get("checks") === "1") {
    const controls = document.createElement("aside");
    controls.setAttribute("aria-label", "合成の保存失敗検証");
    Object.assign(controls.style, { position: "fixed", bottom: "8px", right: "8px", zIndex: "5000", background: "white", color: "#111", padding: "6px", border: "1px solid #bbb", borderRadius: "8px", fontSize: "12px", maxWidth:"calc(100vw - 16px)" });
    controls.append("合成検証：実ファイル変更なし ");
    for (const [label, fail] of [["議事録保存失敗を合成", true], ["議事録保存を復帰", false]]) {
      const button = document.createElement("button"); button.textContent = label;
      button.onclick = () => { window.__mon.failMinutesSave = fail; };
      controls.append(button);
    }
    document.body.append(controls);
    const themeLabel = document.createElement("label"); themeLabel.textContent = "別ウィンドウのテーマ（合成） ";
    const theme = document.createElement("select"); theme.setAttribute("aria-label","別ウィンドウのテーマ（合成）");
    for (const [value,label] of [["","未変更"],["light","ライト"],["dark","ダーク"]]) { const option = document.createElement("option"); option.value=value; option.textContent=label; theme.append(option); }
    theme.onchange = () => { if (theme.value) window.__mon.changeThemeFromOtherWindow(theme.value); };
    themeLabel.append(theme); controls.append(themeLabel);
    const state = document.createElement("output"); state.setAttribute("aria-label", "合成の保存状態"); controls.append(state);
    const documentState = document.createElement("output"); documentState.setAttribute("aria-label", "合成の文書整合性"); controls.append(documentState);
    const baseline = { draft: JSON.stringify(window.__mon.drafts.a.payload), generated: JSON.stringify(window.__mon.views.a.content) };
    const delayLabel = document.createElement("label"); delayLabel.textContent = "保存応答遅延（合成） ";
    const delay = document.createElement("select"); delay.setAttribute("aria-label", "保存応答遅延（合成）");
    for (const [value, label] of [["0", "なし"], ["500", "0.5秒"], ["3000", "3秒"]]) { const option = document.createElement("option"); option.value = value; option.textContent = label; delay.append(option); }
    delay.onchange = () => { window.__mon.minutesSaveDelay = Number(delay.value); };
    delayLabel.append(delay); controls.append(delayLabel);
    const exportLabel = document.createElement("label"); exportLabel.textContent = "出力結果（合成） ";
    const exportMode = document.createElement("select"); exportMode.setAttribute("aria-label", "出力結果（合成）");
    for (const [value,label] of [["cancel","取消"],["success","成功"],["fail","失敗"]]) { const option=document.createElement("option"); option.value=value; option.textContent=label; exportMode.append(option); }
    exportMode.onchange = () => { window.__mon.exportMode = exportMode.value; };
    exportLabel.append(exportMode); controls.append(exportLabel);
    const lostExportLabel=document.createElement("label"); lostExportLabel.textContent="出力応答喪失（合成） ";
    const lostExport=document.createElement("input"); lostExport.type="checkbox"; lostExport.setAttribute("aria-label","出力応答喪失（合成）");
    lostExport.onchange=()=>{window.__mon.lostExportResponse=lostExport.checked;}; lostExportLabel.append(lostExport); controls.append(lostExportLabel);
    const publicationLabel=document.createElement("label"); publicationLabel.textContent="採用・復元応答（合成） ";
    const publicationMode=document.createElement("select"); publicationMode.setAttribute("aria-label","採用・復元応答（合成）");
    for (const [value,label] of [["normal","通常"],["lost","応答喪失"],["foreign","別対象応答"]]) { const option=document.createElement("option"); option.value=value; option.textContent=label; publicationMode.append(option); }
    publicationMode.onchange=()=>{window.__mon.lostPublicationResponse=publicationMode.value==="lost";window.__mon.foreignPublicationReceipt=publicationMode.value==="foreign";};
    publicationLabel.append(publicationMode); controls.append(publicationLabel);
    const complete=document.createElement("button"); complete.textContent="生成完了を合成"; complete.onclick=()=>window.__mon.complete(); controls.append(complete);
    const publicationState=document.createElement("output"); publicationState.setAttribute("aria-label","合成の出力・採用状態"); controls.append(publicationState);
    const close = document.createElement("button"); close.textContent = "合成操作を閉じる";
    close.onclick = () => { controls.hidden = true; }; controls.append(close);
    const update = () => {
      state.textContent = `保存失敗: ${window.__mon.failMinutesSave ? "有効" : "無効"} · 保存要求: ${window.__mon.calls.filter(call => call.cmd === "save_minutes_draft").length} · 元発言読込: ${window.__mon.calls.filter(call => call.cmd === "get_minutes_evidence_source").length}`;
      const integrity = minutesFixtureIntegrity(window.__mon, baseline);
      documentState.textContent = `保存済みブロック: ${integrity.blocks} · 項目: ${integrity.items} · 根拠: ${integrity.evidence} · Undo要求: ${integrity.undoRequests} · 生成原文: ${integrity.originalGenerated ? "保持" : "変更"} · 初期編集文書とのJSON一致: ${integrity.originalDraft ? "一致" : "不一致"}`;
      const count=cmd=>window.__mon.calls.filter(call=>call.cmd===cmd).length;
      publicationState.textContent=`現在版: ${window.__mon.current} · 直前版: ${window.__mon.previous} · 復元要求: ${count("restore_minutes_version")} · 採用要求: ${count("adopt_minutes_candidate")} · 出力要求: ${count("export_minutes_markdown")} · 出力結果読込: ${count("get_minutes_export_status")} · 出力履歴: ${window.__mon.exports.map(record=>record.state).join(",") || "なし"}`;
    };
    update(); window.setInterval(update, 250);
  }
  await import("../src/main.tsx");
} else {
  await import("./browser-fixture.js");
  if (new URLSearchParams(location.search).get("checks") === "1") {
    const controls = document.createElement("aside");
    controls.setAttribute("aria-label", "合成の文字起こし保存検証");
    Object.assign(controls.style, { position:"fixed", bottom:"8px", right:"8px", zIndex:"5000", background:"white", color:"#111", padding:"6px", border:"1px solid #bbb", borderRadius:"8px", fontSize:"12px" });
    controls.append("合成検証：実ファイル変更なし ");
    for (const [label, fail] of [["文字起こし保存失敗を合成", true], ["文字起こし保存を復帰", false]]) {
      const button = document.createElement("button"); button.textContent = label;
      button.onclick = () => { window.monBrowserFixture.failSave = fail; };
      controls.append(button);
    }
    const themeLabel=document.createElement("label"); themeLabel.textContent="別ウィンドウのテーマ（合成） ";
    const theme=document.createElement("select"); theme.setAttribute("aria-label","別ウィンドウのテーマ（合成）");
    for(const [value,label] of [["","未変更"],["light","ライト"],["dark","ダーク"]]) { const option=document.createElement("option");option.value=value;option.textContent=label;theme.append(option); }
    theme.onchange=()=>{if(theme.value){const f=window.monBrowserFixture;f.uiPreferences??={id:"ui_preferences",payload:{schemaVersion:1,theme:"system"},lockVersion:0};f.uiPreferences.payload.theme=theme.value;f.uiPreferences.lockVersion++;void f.emitUiPreferences();}};
    themeLabel.append(theme);controls.append(themeLabel);
    const audioLabel=document.createElement("label");audioLabel.textContent="全体音声の応答（合成） ";
    const audioMode=document.createElement("select");audioMode.setAttribute("aria-label","全体音声の応答（合成）");
    for(const [value,label] of [["normal","正常"],["fail","失敗"],["hold","遅延"]]) {const option=document.createElement("option");option.value=value;option.textContent=label;audioMode.append(option);}
    audioMode.onchange=()=>{const audio=window.monBrowserFixture.meetingAudio;audio.failModes=audioMode.value==="fail"?["both","mic","system"]:[];audio.holdModes=audioMode.value==="hold"?["both","mic","system"]:[];};
    audioLabel.append(audioMode);controls.append(audioLabel);
    const protocolLabel=document.createElement("label");protocolLabel.textContent="音声準備プロトコル（合成） ";
    const protocol=document.createElement("select");protocol.setAttribute("aria-label","音声準備プロトコル（合成）");
    for(const [value,label] of [["legacy","従来"],["strict","ID照合"],["responsive","ID照合・即時取消"],["register","ID登録遅延"],["prepareLost","準備応答喪失"],["receiptLost","受取応答喪失"]]) {const option=document.createElement("option");option.value=value;option.textContent=label;protocol.append(option);}
    protocol.onchange=()=>{const f=window.monBrowserFixture;if(Object.keys(f.preparation.reservations).length||Object.keys(f.meetingAudio.sessions).length)return;Object.assign(f.preparation,{strict:protocol.value!=="legacy",responsive:protocol.value==="responsive",holdRegister:protocol.value==="register",losePrepare:protocol.value==="prepareLost",loseReceipt:protocol.value==="receiptLost"});};
    protocolLabel.append(protocol);controls.append(protocolLabel);
    const releaseRegistration=document.createElement("button");releaseRegistration.textContent="遅延ID登録を返す（合成）";releaseRegistration.onclick=()=>{for(const entry of window.monBrowserFixture.preparation.registerPending)entry.release();};controls.append(releaseRegistration);
    const release=document.createElement("button");release.textContent="遅延音声応答を返す（合成）";release.onclick=()=>{for(const entry of window.monBrowserFixture.meetingAudio.pending)entry.release();};controls.append(release);
    const audioState=document.createElement("output");audioState.setAttribute("aria-label","合成の音声準備状態");controls.append(audioState);
    const ownershipState=document.createElement("output");ownershipState.setAttribute("aria-label","合成の音声所有状態");controls.append(ownershipState);
    const update=()=>{const f=window.monBrowserFixture,calls=f.calls,count=command=>calls.filter(c=>c.command===command).length;audioState.textContent=`全体準備: ${count("prepare_meeting_playback")} · 発言準備: ${count("prepare_utterance_playback")} · 音声読込: ${count("read_playback_chunk")} · 解放: ${count("close_playback")} · 本文保存: ${count("save_transcript_draft")}`;ownershipState.textContent=`ID登録: ${count("register_playback_preparation")} · 取消: ${count("cancel_playback_preparation")} · 受取: ${count("finish_playback_preparation")} · 準備所有: ${Object.keys(f.preparation.reservations).length} · 音声所有: ${Object.keys(f.meetingAudio.sessions).length}`;protocol.disabled=Object.keys(f.preparation.reservations).length>0||Object.keys(f.meetingAudio.sessions).length>0;};
    update();window.setInterval(update,200);
    const close = document.createElement("button"); close.textContent = "合成操作を閉じる";
    close.onclick = () => { controls.hidden = true; }; controls.append(close);
    document.body.append(controls);
  }
}
