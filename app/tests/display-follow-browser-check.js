// Real production UI and silent AudioContext playback, synthetic native IPC.
async (page, capturePath, detailUI) => {
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const start=dock.getByRole("button",{name:"会議全体を再生",exact:true});
  const size=dock.getByRole("combobox",{name:"本文の文字サイズ",exact:true});
  const follow=dock.getByRole("checkbox",{name:"再生位置に追従",exact:true});
  const mode=dock.getByRole("combobox",{name:"会議全体の再生トラック",exact:true});
  const slider=player.getByRole("slider",{name:"会議全体の再生位置",exact:true});
  const speed=player.getByRole("combobox",{name:"速度",exact:true});
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true});
  const port=editor.getByRole("region",{name:"発言本文",exact:true});
  const resume=editor.getByRole("button",{name:"再生位置への追従を再開",exact:true});
  const row=i=>port.locator(`[data-utterance-id="speech-${i}"]`);
  const pages=editor.locator(".transcript-pages");
  const nav=page.getByRole("navigation",{name:"メインナビゲーション",exact:true});
  const count=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command).length,command);
  const details=async()=>{const summary=dock.getByText("再生トラック",{exact:true});if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();};
  const boot=async()=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(()=>window.monBrowserFixture.setUtterances(Array.from({length:103},(_,i)=>({id:`speech-${i}`,trackId:i%2?"system":"microphone",startUs:i*1000000,endUs:(i+1)*1000000,recognizedText:`発言${i} 点検日を確認します。`,effectiveText:`発言${i} 点検日を確認します。`,attributionState:"assigned",speakerIds:[i%2?"speaker-b":"speaker-a"]}))));
    await page.getByRole("button",{name:"開く",exact:true}).first().click();await row(0).waitFor();await details();
    await page.waitForFunction(()=>!document.querySelector(".text-size-choice select")?.disabled);
  };
  const play=async()=>{await start.click();await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();};
  const pause=async()=>{await player.getByRole("button",{name:"一時停止",exact:true}).click();await player.getByRole("button",{name:"再生",exact:true}).waitFor();};
  const expectPage=async text=>{await page.waitForFunction(text=>document.querySelector(".transcript-pages")?.textContent.includes(text),text);};
  const expectFont=async value=>page.waitForFunction(value=>getComputedStyle(document.documentElement).getPropertyValue("--reading-font-size").trim()===`${value}px`,value);
  const syntheticPosition=async(timeUs,patch={})=>page.evaluate(async({timeUs,patch})=>{
    const {playbackTimeline}=await import("/src/playback-follow.ts");
    const current=playbackTimeline.snapshot();if(!current)throw Error("missing production timeline");
    playbackTimeline.publish({...current,timeUs,playing:true,...patch});
  },{timeUs,patch});

  await boot();
  check(await size.inputValue()==="15"&&!await follow.isChecked()&&await count("save_ui_preferences")===0,"legacy defaults caused implicit writes or follow enabled");
  results.push("legacy theme-only preferences display 15px and following OFF without a migration write");
  await play();await pause();await slider.fill("960000");await speed.selectOption("1.5");
  await player.evaluate(e=>{window.displayPlayer=e;});
  const prepares=await count("prepare_meeting_playback");
  for(const value of [13,18,20,15]){
    await size.selectOption(String(value));await expectFont(value);
    check(await row(0).locator(".utterance-text").evaluate((e,value)=>parseFloat(getComputedStyle(e).fontSize)===value,value),"transcript body did not apply confirmed size");
    check(await slider.inputValue()==="960000"&&await speed.inputValue()==="1.5"&&await player.evaluate(e=>e===window.displayPlayer),"font recreated player or changed paused position/rate");
  }
  check(await count("prepare_meeting_playback")===prepares&&await count("save_transcript_draft")===0,"font prepared or published document");
  results.push("all four confirmed sizes affect transcript body and keep paused full-player DOM, exact frame, rate and native token");
  await follow.check();await page.waitForFunction(()=>window.monBrowserFixture.uiPreferences?.payload.followPlayback===true);
  await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption("dark");
  await page.waitForFunction(()=>document.documentElement.dataset.theme==="dark");
  check(await page.evaluate(()=>{const p=window.monBrowserFixture.uiPreferences.payload;return p.schemaVersion===2&&p.textSize===15&&p.followPlayback===true&&p.theme==="dark";}),"theme erased display fields");
  check(await slider.inputValue()==="960000"&&await speed.inputValue()==="1.5","following preference or theme changed paused player");
  results.push("explicit upgrade stores the complete v2 payload; later theme saves retain text size and follow preference");
  await follow.uncheck();await page.waitForFunction(()=>window.monBrowserFixture.uiPreferences.payload.followPlayback===false);
  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();
  await row(0).getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=row(0).getByRole("textbox",{name:"00:00:00の発言",exact:true});
  await field.fill("文字サイズ変更でも未確定の点検日を保持します。");await field.evaluate(e=>{window.displayField=e;e.setSelectionRange(2,6);});
  const saves=await count("save_transcript_draft");
  await size.selectOption("20");await expectFont(20);
  check(await field.evaluate(e=>e===window.displayField&&e.selectionStart===2&&e.selectionEnd===6&&parseFloat(getComputedStyle(e).fontSize)===20)&&await field.inputValue()==="文字サイズ変更でも未確定の点検日を保持します。"&&await count("save_transcript_draft")===saves,"font lost or confirmed editor buffer");
  results.push("font change preserves the unconfirmed text field, content and selection without publishing a transcript edit");

  await boot();await page.evaluate(()=>{window.monBrowserFixture.uiMode="delay";});await size.selectOption("20");
  await page.waitForFunction(()=>!!window.monBrowserFixture.releaseUiSave);
  check(await size.isDisabled()&&await size.inputValue()==="20"&&await row(0).locator(".utterance-text").evaluate(e=>getComputedStyle(e).fontSize==="15px"),"pending font was applied or overlapping save allowed");
  await page.evaluate(()=>{window.monBrowserFixture.uiMode="normal";window.monBrowserFixture.releaseUiSave();});await expectFont(20);
  results.push("pending selection is shown but reading CSS changes only after a confirmed save");
  await page.evaluate(()=>{window.monBrowserFixture.uiMode="fail";});await size.selectOption("13");
  const retry=page.getByRole("button",{name:"文字サイズ・追従設定の保存を再試行",exact:true}).first();await retry.waitFor();
  check(await row(0).locator(".utterance-text").evaluate(e=>getComputedStyle(e).fontSize==="20px")&&await size.isDisabled(),"failed font overwrote confirmed CSS or allowed new save");
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture;f.uiMode="normal";f.uiPreferences={...f.uiPreferences,lockVersion:f.uiPreferences.lockVersion+1,payload:{schemaVersion:2,theme:"dark",textSize:18,followPlayback:true}};
    await f.emitUiPreferences();
  });
  await expectFont(18);await retry.click();await expectFont(13);
  check(await page.evaluate(()=>{const p=window.monBrowserFixture.uiPreferences.payload;return p.theme==="dark"&&p.followPlayback===true&&p.textSize===13;}),"explicit retry erased another window's theme/follow changes");
  results.push("failure keeps confirmed rendering; explicit retry applies only requested size atop another window's newer theme and follow setting");

  await boot();await page.evaluate(()=>{window.monBrowserFixture.uiMode="lostUnreadable";});await size.selectOption("20");
  const confirm=page.getByRole("button",{name:"表示設定の結果を確認",exact:true});await confirm.waitFor();
  check(await size.isDisabled()&&await row(0).locator(".utterance-text").evaluate(e=>getComputedStyle(e).fontSize==="15px")&&await count("save_ui_preferences")===1,"unreadable reply applied guessed result or retried write");
  await page.evaluate(()=>{window.monBrowserFixture.uiReadFail=false;window.monBrowserFixture.uiMode="normal";});await confirm.click();await expectFont(20);
  check(await count("save_ui_preferences")===1&&await size.isEnabled(),"result confirmation rewrote preferences");
  results.push("lost save plus unreadable result freezes new choices; explicit read confirms the committed size without a second write");

  await boot();await play();await slider.fill("3600000");await pause();
  check((await pages.innerText()).includes("1 / 3")&&!await resume.count(),"default OFF moved transcript");
  await follow.check();await page.waitForFunction(()=>window.monBrowserFixture.uiPreferences?.payload.followPlayback===true);
  await player.getByRole("button",{name:"再生",exact:true}).click();await slider.focus();await slider.fill("3600000");await expectPage("2 / 3");
  await page.waitForFunction(()=>{const port=document.querySelector('.utterance-list'),row=port.querySelector('[data-utterance-id="speech-75"]');if(!row)return false;const p=port.getBoundingClientRect(),r=row.getBoundingClientRect();return r.top>=p.top&&r.top<p.bottom;});
  check(await slider.evaluate(e=>document.activeElement===e)&&await page.locator(".content").evaluate(e=>e.scrollTop===0),"follow stole focus or scrolled workspace chrome");
  results.push("actual silent full playback seeks to page 2 and the current row, preserving slider focus and scrolling only transcript body; OFF never moves it");
  await port.hover();await page.mouse.wheel(0,-350);await resume.waitFor();
  await slider.fill("4800000");await pause();
  check((await pages.innerText()).includes("2 / 3"),"manual wheel failed to suspend page following");
  await mode.selectOption("system");await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await resume.isVisible()&&await slider.inputValue()==="4800000","track change reset manual suspension or paused checkpoint");
  await player.getByRole("button",{name:"再生",exact:true}).click();await resume.click();await expectPage("3 / 3");await pause();
  check(Number(await slider.inputValue())>=4800000&&await speed.inputValue()==="1","resume restarted audio or changed rate");
  results.push("manual wheel suspends even cross-page seeks; track switching retains suspension, frame and paused state; explicit resume follows without restarting audio");
  await page.evaluate(()=>{window.monBrowserFixture.meetingAudio.holdModes=["mic"];});await mode.selectOption("mic");
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);
  await port.hover();await page.mouse.wheel(0,-150);await resume.waitFor();
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.meetingAudio.holdModes=[];f.meetingAudio.pending[0].release();});
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await resume.isVisible(),"manual scroll during preparation was forgotten on track completion");
  results.push("manual scrolling during the timeline-release gap of native track preparation also suspends the same session");
  await pages.getByRole("button",{name:"前へ",exact:true}).click();await resume.waitFor();check((await pages.innerText()).includes("2 / 3"),"manual page click was immediately overridden");
  await play();await expectPage("1 / 3");check(await resume.count()===0,"new explicit playback did not clear temporary suspension");await pause();
  results.push("manual pagination suspends following; a new explicit full playback resets temporary suspension and returns to its actual start");
  await port.focus();await port.press("PageDown");await resume.waitFor();
  check(await mode.inputValue()==="mic","keyboard suspension fixture lost its explicit microphone selection");
  await syntheticPosition(75000000);check((await pages.innerText()).includes("1 / 3"),"manual keyboard scroll did not suspend synthetic current-time following");
  await resume.click();
  check((await pages.innerText()).includes("1 / 3"),"microphone following picked the simultaneous PC-only row");
  await syntheticPosition(76000000);await expectPage("2 / 3");
  results.push("focused transcript PageDown suspends following; explicit resume preserves the selected track, ignores PC-only speech and follows the next microphone position");

  // Paused audio stops the playback publisher. Inject current-time notifications
  // solely to isolate consumer guards; these are not native/audio output tests.
  await boot();await follow.check();await play();await pause();
  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();await row(0).getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  await field.fill("編集中の入力は追従で移動しません。");await field.evaluate(e=>{window.followField=e;e.setSelectionRange(1,5);});
  const editSaves=await count("save_transcript_draft");await syntheticPosition(75000000);
  check((await pages.innerText()).includes("1 / 3")&&await field.evaluate(e=>e===window.followField&&e.selectionStart===1&&e.selectionEnd===5)&&await count("save_transcript_draft")===editSaves,"following moved or published active editing");
  results.push("synthetic position changes never move pages, remount inputs or publish text while an utterance edit is active");

  await boot();await follow.check();await play();await pause();
  await row(0).locator(".utterance-text").evaluate(e=>{const r=document.createRange();r.setStart(e.firstChild,1);r.setEnd(e.firstChild,4);getSelection().removeAllRanges();getSelection().addRange(r);});
  await syntheticPosition(75000000);check((await pages.innerText()).includes("1 / 3"),"following overrode text selection");
  await page.evaluate(()=>getSelection().removeAllRanges());await syntheticPosition(75100000);await expectPage("2 / 3");
  results.push("non-collapsed reading selection blocks synthetic following; collapsing it allows the next position notification");
  const search=editor.getByRole("searchbox",{name:"発言を検索",exact:true});await search.fill("発言0 ");
  await page.getByRole("banner").click();await syntheticPosition(75000000);
  check(await search.inputValue()==="発言0 "&&await row(0).isVisible()&&await row(75).count()===0,"following cleared search or invented an absent match");
  results.push("following never clears search or invents a nearest matching utterance outside the filtered rows");

  await boot();await follow.check();await play();await pause();
  await detailUI.openTranscriptTools();await editor.getByRole("button",{name:"編集を開始",exact:true}).click();
  await detailUI.openSpeakerPanel();await row(0).getByRole("button",{name:"発言を手動分割",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"発言の手動分割",exact:true});await dialog.waitFor();
  await syntheticPosition(75000000);check((await pages.innerText()).includes("1 / 3"),"open confirmation did not block following");
  await dialog.getByRole("button",{name:"キャンセル",exact:true}).click();await syntheticPosition(75200000);await expectPage("2 / 3");
  results.push("a real manual-split dialog blocks synthetic following; closing it allows a later timeline notification");

  await boot();await follow.check();await play();await pause();
  await page.getByRole("tab",{name:"議事録",exact:true}).click();await syntheticPosition(75000000);
  check((await page.getByRole("region",{name:"文字起こし編集",exact:true,includeHidden:true}).locator(".transcript-pages").textContent()).includes("1 / 3"),"hidden transcript followed behind minutes");
  await page.getByRole("tab",{name:"文字起こし",exact:true}).click();await expectPage("2 / 3");
  results.push("hidden transcript does not move behind the minutes tab; returning consumes the current valid meeting position");

  await boot();await play();await pause();await slider.fill("960000");await speed.selectOption("1.5");
  for(const width of [1280,960,360])for(const theme of ["light","dark"])for(const value of [13,20]){
    await page.setViewportSize({width,height:640});await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    await size.selectOption(String(value));await expectFont(value);
    check(await dock.evaluate(e=>{const r=e.getBoundingClientRect();return r.bottom<=innerHeight+1&&e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth;}),"display controls overflow or hide dock");
    await player.getByRole("button",{name:"再生",exact:true}).focus();
    check(await player.getByRole("button",{name:"再生",exact:true}).evaluate(e=>e.getBoundingClientRect().bottom<=innerHeight)&&await slider.inputValue()==="960000"&&await speed.inputValue()==="1.5","font or narrow layout hides playback or changes checkpoint");
    if(value===20)await page.screenshot({path:`display-follow-${theme}-${width}.png`});
  }
  results.push("13px/20px, both themes and 1280/960/360 widths keep dock bounded and primary playback action visible without changing paused frame/rate");
  check(await count("start_capture")===0,"synthetic verification started recording");
  return {passed:results.length,results};
}
