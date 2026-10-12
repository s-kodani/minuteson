// Synthetic browser preview only. No native DB/models/audio/file writes.
if (new URLSearchParams(location.search).get("view") === "minutes") {
  await import("./plain-minutes-production-browser-fixture.js");
  if (new URLSearchParams(location.search).get("checks") === "1") {
    const controls = document.createElement("aside");
    controls.setAttribute("aria-label", "合成の保存失敗検証");
    Object.assign(controls.style, { position:"fixed", bottom:"8px", right:"8px", zIndex:"5000", background:"white", color:"#111", padding:"8px", border:"1px solid #bbb", borderRadius:"8px", fontSize:"12px", maxWidth:"calc(100vw - 16px)", display:"flex", gap:"8px", flexWrap:"wrap" });
    controls.append("合成検証：実ファイル変更なし ");
    for (const [label, mode] of [["議事録保存失敗を合成","failure"],["議事録保存を復帰","success"],["保存成功応答の喪失を合成","lost"]]) {
      const button=document.createElement("button");button.textContent=label;
      button.onclick=()=>{window.__mon.plainMode=mode;};controls.append(button);
    }
    const complete=document.createElement("button");complete.textContent="生成完了を合成";
    complete.onclick=()=>{if(window.__mon.jobs.length)window.__mon.completePlain();};controls.append(complete);
    const state=document.createElement("output");state.setAttribute("aria-label","合成の保存状態");controls.append(state);
    const update=()=>{const data=window.__mon;state.textContent=`保存状態: ${data.plainMode} · 保存要求: ${data.plainCalls.filter(c=>c.cmd==="save_minutes_markdown").length}`;};
    update();window.setInterval(update,250);
    const close = document.createElement("button"); close.textContent = "合成操作を閉じる";
    close.onclick = () => { controls.hidden = true; }; controls.append(close);
    document.body.append(controls);
  }
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
