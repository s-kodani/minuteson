// Production meeting dock, synthetic IPC and silent PCM only.
async (page, capturePath, detailUI) => {
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const start=dock.getByRole("button",{name:"会議全体を再生",exact:true});
  const mode=dock.getByRole("combobox",{name:"会議全体の再生トラック",exact:true});
  const slider=player.getByRole("slider",{name:"会議全体の再生位置",exact:true});
  const speed=player.getByRole("combobox",{name:"速度",exact:true});
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true});
  const row=editor.getByRole("article",{name:"00:00:00の発言行",exact:true});
  const nav=page.getByRole("navigation",{name:"メインナビゲーション",exact:true});
  const boot=async source=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    if(source!==undefined)await page.evaluate(source=>window.monBrowserFixture.setMeetingSource("meeting-0",source),source);
    await page.locator(".meeting-name").first().click();
    await editor.getByRole("button",{name:"編集を開始",exact:true}).waitFor();
  };
  const details=async()=>{const summary=dock.getByText("再生トラック",{exact:true});if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();};
  const play=async()=>{await start.click();await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();};
  const pause=async()=>{await player.getByRole("button",{name:"一時停止",exact:true}).click();await player.getByRole("button",{name:"再生",exact:true}).waitFor();};
  const count=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command).length,command);
  const release=async index=>{const token=await page.evaluate(index=>{const pending=window.monBrowserFixture.meetingAudio.pending[index];pending.release();return pending.token;},index);await page.waitForFunction(token=>window.monBrowserFixture.calls.some(c=>c.command==="close_playback"&&c.args.token===token),token);return token;};

  await boot();
  check(await start.isEnabled()&&await player.count()===0&&await count("prepare_meeting_playback")===0,"mount auto-prepared or played meeting");
  await details();check(await mode.inputValue()==="both"&&await mode.locator("option").count()===3,"recorded source modes invalid");
  await mode.selectOption("mic");await mode.selectOption("both");
  check(await count("prepare_meeting_playback")===0,"idle mode selection performed native preparation");
  results.push("idle full-meeting action and explicit source modes never prepare or play on mount or selection alone");
  await play();await pause();
  check((await player.innerText()).includes("60:00")&&(await dock.innerText()).includes("会議全体の音声")&&(await player.innerText()).includes("同期精度は未検証"),"whole timeline or unverified warning missing");
  check(await dock.getByRole("slider",{name:"発言内の再生位置",exact:true}).count()===0,"full meeting is mislabeled as an utterance range");
  results.push("whole meeting uses a distinct 60-minute range and unverified synchronization instead of an utterance duration");
  await slider.fill("960000");await speed.selectOption("1.5");await mode.selectOption("system");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await slider.inputValue()==="960000"&&await speed.inputValue()==="1.5"&&(await player.innerText()).includes("PC音声"),"paused mode change lost frame, speed or track");
  results.push("paused mic/system/both switch preserves the exact meeting frame, speed and paused state");
  await player.getByRole("button",{name:"再生",exact:true}).click();await mode.selectOption("mic");
  await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();await pause();
  check(Number(await slider.inputValue())>=960000&&await speed.inputValue()==="1.5","playing switch restarted the meeting or changed speed");
  results.push("playing track switch resumes from the current meeting position, not from zero");

  await slider.fill("1440000");await speed.selectOption("0.75");
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["system"]);await mode.selectOption("system");
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);await mode.selectOption("both");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await slider.inputValue()==="1440000"&&await speed.inputValue()==="0.75","rapid switch lost the last confirmed checkpoint");
  await release(0);
  check(await slider.inputValue()==="1440000"&&await speed.inputValue()==="0.75"&&(await player.innerText()).includes("マイク＋PC音声"),"late source replaced or released newer playback");
  results.push("rapid switches carry the last confirmed checkpoint through pending preparation; late old tokens close without changing the new owner");

  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();
  await row.getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=row.getByRole("textbox",{name:"00:00:00の発言",exact:true});
  await field.fill("全体再生の切替で未確定本文を保存しません。");await field.evaluate(e=>{window.meetingAudioField=e;e.setSelectionRange(2,6);});
  const saves=await count("save_transcript_draft");await slider.fill("1920000");await speed.selectOption("2");await mode.selectOption("mic");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await field.inputValue()==="全体再生の切替で未確定本文を保存しません。"&&await field.evaluate(e=>e===window.meetingAudioField&&e.selectionStart===2&&e.selectionEnd===6)&&await count("save_transcript_draft")===saves,"full playback changed text, DOM, selection or publication");
  results.push("whole-meeting seek, speed and track controls retain unconfirmed text and selection without publishing it");

  await boot();await details();await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["both"]);await start.click();
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);
  await player.getByRole("button",{name:"再生準備を取り消す",exact:true}).click();await release(0);
  check(await player.count()===0&&await count("read_playback_chunk")===0,"cancelled preparation scheduled audio");
  results.push("explicit cancellation discards and closes a late token without reading or scheduling audio");

  await boot();await play();const fullToken=await page.evaluate(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions)[0]);
  await row.getByRole("button",{name:"音声を再生",exact:true}).click();
  await dock.getByRole("group",{name:"発言の音声再生",exact:true}).getByRole("button",{name:"一時停止",exact:true}).waitFor();
  await page.waitForFunction(token=>window.monBrowserFixture.calls.some(c=>c.command==="close_playback"&&c.args.token===token),fullToken);
  check(await player.count()===0,"utterance and whole playback controls coexist");
  await play();await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="close_playback"&&c.args.token==="audio-token"));
  check(await dock.getByRole("group",{name:"発言の音声再生",exact:true}).count()===0,"full playback did not release utterance owner");
  results.push("utterance and full meeting replace each other exclusively and release the previous native token");

  await boot();await details();await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["both"]);await start.click();
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);
  await page.evaluate(async()=>{const {stopAllPlayback}=await import("/src/playback-barrier.ts");window.meetingBarrierFinished=false;window.meetingBarrierJob=stopAllPlayback().then(()=>{window.meetingBarrierFinished=true;});});
  check(!await page.evaluate(()=>window.meetingBarrierFinished),"global release barrier ignored pending meeting preparation");
  await release(0);await page.waitForFunction(()=>window.meetingBarrierFinished);
  check(await player.count()===0&&await count("read_playback_chunk")===0,"global stop revived meeting audio");
  results.push("data-operation release barrier waits for pending whole-meeting preparation and native token close");

  await boot();await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["both"]);await start.click();
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);await nav.getByRole("button",{name:"設定",exact:true}).click();
  await dock.waitFor({state:"hidden"});await release(0);await nav.getByRole("button",{name:"会議一覧",exact:true}).click();await page.locator(".meeting-name").first().click();await start.waitFor();
  check(await player.count()===0&&await count("prepare_meeting_playback")===1,"settings return adopted late audio or prepared automatically");
  results.push("settings unmount closes late preparation and return never revives or auto-prepares it");
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=[]);
  await play();await detailUI.openMeetingTools();await page.locator(".meeting-name").nth(1).click();
  await page.waitForFunction(()=>document.querySelector(".meeting-playback-content")?.dataset.meetingId==="meeting-1");
  check(await player.count()===0&&(await dock.innerText()).includes("会議B"),"different meeting inherited the previous full player");
  results.push("meeting change releases full playback and changes the dock owner without auto-playing");

  await boot();await page.evaluate(()=>window.monBrowserFixture.meetingAudio.failModes=["both"]);await start.click();await player.getByRole("alert").waitFor();
  check((await player.innerText()).includes("欠損")&&await count("read_playback_chunk")===0,"native rejection was hidden or played audio");
  results.push("missing native meeting audio is a visible failure, never a fallback or apparent successful replay");
  await boot();await page.evaluate(()=>window.monBrowserFixture.meetingAudio.corrupt={trackKind:"mic"});await start.click();await player.getByRole("alert").waitFor();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="close_playback"));
  check(await count("read_playback_chunk")===0,"wrong-kind meeting token was played");
  results.push("invalid meeting output contract releases its token before any PCM read");
  await boot();await play();await pause();await slider.fill("1920000");await details();
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.endUs=7200000000);await mode.selectOption("system");await player.getByRole("alert").waitFor();
  check((await player.innerText()).includes("最初から選び直してください"),"changed source duration was silently clamped and resumed");
  results.push("changed native timeline refuses checkpoint reuse instead of clamping or restarting silently");

  await boot("audio_import");await details();check(await mode.locator("option").count()===1&&await mode.inputValue()==="imported","imported source fabricated mic/system tracks");
  await play();await pause();await slider.fill("960000");await speed.selectOption("1.25");
  await player.evaluate(e=>{window.meetingAudioNode=e;});
  for(const width of [1280,960,360])for(const theme of ["light","dark"]){
    await page.setViewportSize({width,height:640});await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    check(await slider.inputValue()==="960000"&&await speed.inputValue()==="1.25"&&await player.evaluate(e=>e===window.meetingAudioNode),"resize/theme recreated meeting player or reset state");
    check(await dock.evaluate(e=>{const r=e.getBoundingClientRect();return r.bottom<=innerHeight+1&&e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth&&document.querySelector(".content").getBoundingClientRect().bottom<=r.top+1;}),"whole player dock overlaps or overflows");
    await player.getByRole("button",{name:"再生",exact:true}).focus();
    check(await player.getByRole("button",{name:"再生",exact:true}).evaluate(e=>e.getBoundingClientRect().bottom<=innerHeight),"primary meeting audio action is offscreen");
    await page.screenshot({path:`meeting-audio-${theme}-${width}.png`});
  }
  results.push("imported mono/stereo origin uses only the imported mode; both themes and 1280/960/360 widths preserve player DOM, paused position, speed and bounded dock");
  await boot(null);check(await start.count()===0,"unknown source guessed a playback mode");
  results.push("unknown meeting origin never guesses a full-playback source");
  return {passed:results.length,results};
}
