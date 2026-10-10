// Actual workspace, responsive portal and synthetic IPC. No real recording.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok,message) => { if (!ok) throw Error(message); };
  const editor = page.getByRole("region",{ name:"文字起こし編集",exact:true });
  const drawer = () => page.getByRole("dialog",{ name:"話者一覧",exact:true });
  const open = () => page.getByRole("button",{ name:"話者一覧を開く",exact:true });
  const group = label => editor.getByRole("group",{ name:`${label}の名前編集`,exact:true });
  const name = () => group("話者A").getByRole("textbox",{ name:"話者A",exact:true });
  const body = editor.getByRole("region",{ name:"発言本文",exact:true });
  const writes = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft"));
  const starts = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "start_diarization"));
  const boot = async (configured = false) => {
    await page.setViewportSize({ width:1200,height:820 }); await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(configured => {
      const f = window.monBrowserFixture;
      f.setSpeakers([{ id:"speaker-a",label:"話者A",displayName:"田中" },{ id:"speaker-b",label:"話者B",displayName:"佐藤" },...Array.from({ length:18 },(_,i) => ({ id:`extra-${i}`,label:`追加話者${i}`,displayName:null }))]);
      f.setUtterances(Array.from({ length:103 },(_,i) => ({ id:`speech-${i}`,trackId:"microphone",startUs:i*1_000_000,endUs:(i+1)*1_000_000,recognizedText:`本文${i}です。`,effectiveText:`本文${i}です。`,attributionState:i%3===2?"unknown":"assigned",speakerIds:i%3===2?[]:[i%3===0?"speaker-a":"speaker-b"] })));
      if (configured) Object.assign(f.diarizationPreferences.payload.standard,{ executablePath:"/synthetic/sherpa",segmentationModelPath:"/synthetic/segments.onnx",embeddingModelPath:"/synthetic/embedding.onnx" });
    },configured);
    await page.locator(".meeting-name").first().click();
    await editor.getByRole("button",{ name:"編集を開始",exact:true }).waitFor();
  };
  await boot();
  for (const width of [1280,1200,1120]) for (const theme of ["light","dark"]) {
    await page.setViewportSize({ width,height:640 });
    await page.getByRole("banner").getByRole("combobox",{ name:"表示テーマ",exact:true }).selectOption(theme);
    await page.waitForFunction(t => document.documentElement.dataset.theme === t,theme);
    const bounds = await page.evaluate(() => {
      const panel=document.querySelector(".workspace-panel-shell").getBoundingClientRect(),main=document.querySelector(".transcript-main").getBoundingClientRect(),list=document.querySelector(".utterance-list").getBoundingClientRect(),dock=document.querySelector(".meeting-playback-dock").getBoundingClientRect(),side=document.querySelector(".speaker-panel-body");
      return { width:panel.width,gap:main.left-panel.right,height:list.height,bottom:list.bottom,dock:dock.top,sideScroll:side.scrollHeight>side.clientHeight,overflow:document.documentElement.scrollWidth>innerWidth||document.documentElement.scrollHeight>innerHeight+1 };
    });
    check(bounds.width===216 && bounds.gap===24 && bounds.height>80 && bounds.bottom<=bounds.dock && bounds.sideScroll && !bounds.overflow,`wide layout invalid: ${JSON.stringify(bounds)}`);
    check(await open().isHidden() && await name().isDisabled() && await editor.getByText("名前未設定",{ exact:true }).count()===18,"wide panel missing read-only names/unset labels or exposes drawer trigger");
    await page.screenshot({ path:`speaker-panel-${theme}-${width}.png` });
  }
  results.push("1280/1200/1120 by 640 in both themes: 216px side panel, 24px gap, independent speaker/body scrolling and unset-name labels");
  await editor.getByRole("button",{ name:"田中の発言で絞り込み",exact:true }).click();
  check(await body.getByRole("article").count()===35 && await editor.getByRole("combobox",{ name:"話者で絞り込み",exact:true }).inputValue()==="speaker-a","speaker filter is not connected to the transcript");
  const search = editor.getByRole("searchbox",{ name:"発言を検索",exact:true }); await search.fill("本文0です");
  check(await body.getByRole("article").count()===1,"text search did not combine with speaker filter");
  await search.fill(""); await editor.getByRole("button",{ name:"話者不明の発言",exact:true }).click();
  check(await body.getByRole("article").count()===34,"unknown filter assigned an invented speaker");
  await editor.getByRole("button",{ name:"すべての発言",exact:true }).click();
  check(await body.getByRole("article").count()===50 && (await writes()).length===0,"clearing filters wrote an edit or failed to restore the first page");
  results.push("speaker, unknown and all filters combine with text search without any canonical write");
  await editor.getByRole("button",{ name:"編集を開始",exact:true }).click();
  const row = editor.getByRole("article",{ name:"00:00:00の発言行",exact:true });
  await row.getByRole("button",{ name:"00:00:00の発言を編集",exact:true }).click();
  const text = row.getByRole("textbox",{ name:"00:00:00の発言",exact:true }); await text.fill("絞り込み中も保持する本文");
  await text.evaluate(e => { window.speakerText=e; }); await editor.getByRole("button",{ name:"佐藤の発言で絞り込み",exact:true }).click();
  check(await text.inputValue()==="絞り込み中も保持する本文" && await text.evaluate(e => e===window.speakerText) && (await writes()).length===0,"speaker filtering discarded or confirmed an edited row");
  results.push("speaker-panel filtering retains the same editing row and unconfirmed text outside the filter");

  await boot(true); await editor.getByRole("button",{ name:"編集を開始",exact:true }).click(); await detailUI.openDiarization();
  const diar = () => page.getByRole("region",{ name:"話者分離操作",exact:true });
  const countMode = () => diar().getByRole("combobox",{ name:"マイク 人数",exact:true });
  const fixedCount = () => diar().getByRole("spinbutton",{ name:"マイクの指定人数",exact:true });
  await countMode().selectOption("fixed"); await fixedCount().fill("4");
  await name().fill("幅切替で保持する未確定名");
  await name().evaluate(e => { window.speakerName=e; e.setSelectionRange(2,5); });
  await fixedCount().evaluate(e => { window.speakerCount=e; });
  const reads = await page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command==="get_diarization_preferences").length);
  await page.setViewportSize({ width:960,height:640 }); await drawer().waitFor();
  await page.waitForFunction(() => document.activeElement===window.speakerName);
  check(await name().evaluate(e => e===window.speakerName && e.selectionStart===2 && e.selectionEnd===5) && await name().inputValue()==="幅切替で保持する未確定名" && await fixedCount().evaluate(e => e===window.speakerCount) && await fixedCount().inputValue()==="4","resize recreated fields, selection or local count state");
  check((await writes()).length===0 && await page.evaluate(n => window.monBrowserFixture.calls.filter(c => c.command==="get_diarization_preferences").length===n,reads),"resize confirmed names or duplicated the diarization controller");
  results.push("focused resize opens the drawer while preserving name DOM, selection, count DOM/state and the single diarization controller");
  await detailUI.closeSpeakerPanel(); await open().waitFor();
  check(await open().evaluate(e => e===document.activeElement) && (await writes()).length===0,"closing drawer failed to restore its trigger or published input");
  await open().click(); check(await name().inputValue()==="幅切替で保持する未確定名" && await name().evaluate(e => e===window.speakerName),"drawer reopen discarded input identity");
  for (const args of [{ key:"Escape",isComposing:true },{ key:"Escape",keyCode:229 },{ key:"Escape",repeat:true },{ key:"Enter",repeat:true }]) {
    await name().evaluate((e,args) => e.dispatchEvent(new KeyboardEvent("keydown",{ ...args,bubbles:true,cancelable:true })),args);
    check(await drawer().isVisible() && (await writes()).length===0 && await name().inputValue()==="幅切替で保持する未確定名","IME or a held key closed the drawer or confirmed/cancelled the name");
  }
  await name().press("Enter");
  await group("話者A").getByRole("button",{ name:"確定",exact:true }).waitFor({ state:"hidden" });
  check(await drawer().isVisible() && (await writes()).length===1 && await name().inputValue()==="幅切替で保持する未確定名","Enter closed the drawer or failed explicit name confirmation");
  await name().fill("取消する未確定名"); await name().press("Escape");
  await group("話者A").getByRole("button",{ name:"キャンセル",exact:true }).waitFor({ state:"hidden" });
  check(await drawer().isVisible() && await name().inputValue()==="幅切替で保持する未確定名" && (await writes()).length===1,"name Escape dismissed its drawer or wrote an unpublished name");
  await name().press("Escape"); await drawer().waitFor({ state:"hidden" });
  check(await open().evaluate(e => e===document.activeElement),"Escape without a name session did not restore drawer trigger");
  results.push("drawer close/reopen keeps input; Enter confirms a name, first Escape cancels only the name and subsequent Escape closes with focus restoration");

  await open().click();
  const close = drawer().getByRole("button",{ name:"話者一覧を閉じる",exact:true });
  await close.focus(); await close.press("Shift+Tab");
  check(await page.evaluate(() => !!document.activeElement?.closest(".workspace-drawer[open]")),"reverse Tab escaped through browser chrome");
  await page.keyboard.press("Tab"); check(await close.evaluate(e => e===document.activeElement),"forward Tab did not wrap to drawer dismissal");
  await page.setViewportSize({ width:1200,height:640 }); await drawer().waitFor({ state:"hidden" });
  check(await name().evaluate(e => e===window.speakerName) && await fixedCount().evaluate(e => e===window.speakerCount) && await fixedCount().inputValue()==="4","wide return recreated portal fields or settings");
  results.push("native drawer Tab wrapping includes portalled settings; returning wide preserves the same fields");

  await page.setViewportSize({ width:960,height:640 }); await detailUI.openSpeakerPanel();
  await editor.getByRole("combobox",{ name:"統合元",exact:true }).selectOption("speaker-a");
  await editor.getByRole("combobox",{ name:"統合先",exact:true }).selectOption("speaker-b");
  const merge = editor.getByRole("button",{ name:"話者を統合",exact:true }); await merge.click();
  const choice = () => page.getByRole("dialog",{ name:"話者の統合を確認",exact:true }); await choice().waitFor();
  const before = (await writes()).length;
  await page.keyboard.press("Escape"); await drawer().waitFor();
  check((await writes()).length===before && await merge.evaluate(e => e===document.activeElement),"nested merge cancellation closed its drawer, wrote data or lost focus");
  await merge.click(); await choice().waitFor(); await page.setViewportSize({ width:1200,height:640 });
  await choice().waitFor(); check((await writes()).length===before,"resizing while confirming implicitly accepted merge");
  await page.keyboard.press("Escape"); await choice().waitFor({ state:"hidden" });
  check(await merge.isVisible() && (await writes()).length===before,"resizing discarded an owned choice or wrote a merge");
  results.push("nested merge confirmation cancels back to its drawer; resize preserves pending confirmation without accepting or writing it");

  await boot(true); await detailUI.openDiarization(); await countMode().selectOption("fixed"); await fixedCount().fill("7");
  await fixedCount().evaluate(e => { window.tabCount=e; });
  await page.setViewportSize({ width:960,height:640 }); await detailUI.openSpeakerPanel(); await detailUI.closeSpeakerPanel();
  await page.getByRole("tab",{ name:"議事録",exact:true }).click();
  check(await page.evaluate(() => window.tabCount.isConnected && !window.tabCount.getClientRects().length),"minutes tab destroyed or exposed speaker settings");
  await page.getByRole("tab",{ name:"文字起こし",exact:true }).click();
  check(await open().getAttribute("aria-expanded")==="false","tab return unexpectedly reopened a modal drawer");
  await detailUI.openDiarization(); check(await fixedCount().evaluate(e => e===window.tabCount) && await fixedCount().inputValue()==="7","tab return lost local count state or recreated the controller");
  await detailUI.closeSpeakerPanel(); await page.getByRole("button",{ name:"会議一覧",exact:true }).click();
  await page.locator(".meeting-name").nth(1).click(); await detailUI.openMeetingTools();
  check(await page.locator(".speaker-diarization-tools").count()===0 && await countMode().inputValue()==="auto" && await diar().isVisible(),"meeting without an adopted transcript inherited counts or lost fallback diarization controls");
  results.push("tab return retains the single hidden count editor without reopening a drawer; another meeting resets counts and keeps the no-transcript fallback");

  await boot(true); await detailUI.openDiarization(); await countMode().selectOption("fixed"); await fixedCount().fill("6");
  await fixedCount().evaluate(e => { window.adoptCount=e; });
  const preferenceReads = await page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command==="get_diarization_preferences").length);
  await detailUI.openMeetingTools();
  const analysis = page.getByRole("region",{ name:"解析操作",exact:true });
  await analysis.getByRole("button",{ name:"内容を確認",exact:true }).click();
  await page.getByRole("dialog",{ name:"候補版の採用確認 #2",exact:true }).getByRole("button",{ name:"この候補を採用",exact:true }).click();
  await page.getByRole("dialog",{ name:"文字起こし候補の採用を確認",exact:true }).getByRole("button",{ name:"候補を採用する",exact:true }).click();
  await page.getByRole("dialog",{ name:"候補版の採用確認 #2",exact:true }).waitFor({ state:"hidden" });
  await detailUI.openDiarization();
  check(await fixedCount().evaluate(e => e===window.adoptCount) && await fixedCount().inputValue()==="6" && await page.evaluate(n => window.monBrowserFixture.calls.filter(c => c.command==="get_diarization_preferences").length===n && window.monBrowserFixture.currentId==="candidate",preferenceReads),"candidate adoption recreated diarization controls or lost counts when the transcript host changed");
  results.push("confirmed candidate adoption replaces the transcript host but preserves the same diarization controller, count DOM and unsent count setting");

  await boot(true); await editor.getByRole("button",{ name:"編集を開始",exact:true }).click(); await detailUI.openDiarization();
  await countMode().selectOption("fixed"); await fixedCount().fill("4");
  await name().fill("開始前に保存する話者名"); await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs=500; });
  await diar().getByRole("button",{ name:"話者分離を開始・やり直す",exact:true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(c => c.command==="save_transcript_draft"));
  check((await starts()).length===0,"diarization started before name save acknowledgement");
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(c => c.command==="start_diarization"));
  const request = (await starts())[0].args.request;
  check(request.tracks.find(t => t.trackId==="microphone").numSpeakers===4 && request.tracks.find(t => t.trackId==="system").numSpeakers===null && request.input.meetingId==="meeting-0" && request.input.expectedAnalysisId==="analysis" && request.input.expectedContentHash==="a".repeat(64) && !!request.input.operationId,"fixed count or analysis identity changed");
  check(await page.evaluate(r => r.input.expectedDraft.lockVersion===window.monBrowserFixture.draft.lockVersion && window.monBrowserFixture.draft.payload.speaker_names[0].display_name==="開始前に保存する話者名" && window.monBrowserFixture.currentId==="analysis" && window.monBrowserFixture.candidateVisible,request),"start did not freeze the saved edit or replaced current/candidate version");
  results.push("diarization waits for saved names, then fixes per-track count/auto, saved draft and analysis identity without adopting a candidate");

  await boot(true); await editor.getByRole("button",{ name:"編集を開始",exact:true }).click(); await detailUI.openDiarization();
  await page.setViewportSize({ width:960,height:640 }); await detailUI.openSpeakerPanel();
  await name().fill("失敗時も保持する名前"); await page.evaluate(() => { window.monBrowserFixture.failSave=true; });
  await diar().getByRole("button",{ name:"話者分離を開始・やり直す",exact:true }).click();
  await drawer().getByRole("status").filter({ hasText:"話者名の保存結果を確認" }).waitFor();
  check((await starts()).length===0 && await name().inputValue()==="失敗時も保持する名前","failed flush started diarization or discarded drawer input");
  await page.evaluate(() => { window.monBrowserFixture.failSave=false; });
  await drawer().getByRole("button",{ name:"話者名の保存を再試行",exact:true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.speaker_names[0]?.display_name==="失敗時も保持する名前");
  check((await starts()).length===0,"name-only retry unexpectedly started diarization");
  results.push("failed name flush refuses diarization; drawer retains input and explicit name-save retry never starts analysis");
  await page.evaluate(async () => { const snapshot={ revision:5,phase:"recording",elapsedMs:1000,microphoneConnected:true,systemAudioConnected:true,meetingId:"meeting-0",sessionId:"synthetic-speaker-session",errorMessage:null }; window.monBrowserFixture.recording=snapshot; await window.__TAURI_INTERNALS__.invoke("plugin:event|emit",{ event:"session-changed",payload:snapshot }); });
  check(await diar().getByRole("button",{ name:"話者分離を開始・やり直す",exact:true }).isDisabled() && await countMode().isDisabled() && (await starts()).length===0,"recording failed to block counts and analysis start");
  await detailUI.closeSpeakerPanel();
  for (const theme of ["light","dark"]) {
    await page.getByRole("banner").getByRole("combobox",{ name:"表示テーマ",exact:true }).selectOption(theme); await page.waitForFunction(t => document.documentElement.dataset.theme===t,theme);
    await detailUI.openSpeakerPanel(); await page.screenshot({ path:`speaker-drawer-${theme}-960.png` }); await detailUI.closeSpeakerPanel();
  }
  check(await page.evaluate(() => !window.monBrowserFixture.calls.some(c => /^(start_recording|stop_recording|request_recording_confirmation)$/u.test(c.command))),"speaker controls changed native recording");
  results.push("recording blocks per-track counts/start; both drawer themes remain usable without any real recording command");
  return { passed:results.length,results };
}
