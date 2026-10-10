// Production playback controls with synthetic native reservations and silence.
async page => {
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const start=dock.getByRole("button",{name:"会議全体を再生",exact:true});
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true});
  const row=editor.getByRole("article",{name:"00:00:00の発言行",exact:true});
  const mode=dock.getByRole("combobox",{name:"会議全体の再生トラック",exact:true});
  const count=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command).length,command);
  const boot=async patch=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(patch=>Object.assign(window.monBrowserFixture.preparation,{strict:true,...patch}),patch??{});
    await page.locator(".meeting-name").first().click();
    await editor.getByRole("button",{name:"編集を開始",exact:true}).waitFor();
  };
  const barrier=async()=>page.evaluate(async()=>{
    const {stopAllPlayback}=await import("/src/playback-barrier.ts");
    window.cancelBarrierState="pending";
    window.cancelBarrierJob=stopAllPlayback().then(()=>{window.cancelBarrierState="fulfilled";},e=>{window.cancelBarrierState="rejected";window.cancelBarrierError=String(e);});
  });
  const settled=()=>page.waitForFunction(()=>window.cancelBarrierState!=="pending");
  const released=()=>page.waitForFunction(()=>Object.keys(window.monBrowserFixture.preparation.reservations).length===0&&Object.keys(window.monBrowserFixture.meetingAudio.sessions).length===0);
  const holdMeeting=async()=>{await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["both"]);await start.click();await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);};

  await boot({holdRegister:true});await start.click();
  await page.waitForFunction(()=>window.monBrowserFixture.preparation.registerPending.length===1);
  await player.getByRole("button",{name:"再生準備を取り消す",exact:true}).click();await barrier();
  check(await count("prepare_meeting_playback")===0&&await count("cancel_playback_preparation")===0&&await page.evaluate(()=>window.cancelBarrierState)==="pending","cancel overtook registration or stop finished before registration");
  await page.evaluate(()=>window.monBrowserFixture.preparation.registerPending[0].release());await settled();await released();
  check(await count("prepare_meeting_playback")===0&&await count("read_playback_chunk")===0,"cancelled reservation started expensive preparation or audio");
  results.push("cancellation during reserved-ID registration waits for its response and never starts expensive preparation");

  await boot({responsive:true});await holdMeeting();
  for(const width of [1280,960,360])for(const theme of ["light","dark"]){
    await page.setViewportSize({width,height:640});await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    check(await player.getByRole("button",{name:"再生準備を取り消す",exact:true}).evaluate(e=>{const r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&r.right<=innerWidth;})&&await dock.evaluate(e=>e.scrollWidth<=e.clientWidth+1),"preparation cancellation is offscreen or overflows");
    await page.screenshot({path:`playback-preparation-${theme}-${width}.png`});
  }
  results.push("native preparation cancellation stays visible and bounded at 1280, 960 and 360 pixels in both themes");
  await page.setViewportSize({width:1200,height:900});
  await player.getByRole("button",{name:"再生準備を取り消す",exact:true}).click();await barrier();await settled();await released();
  const scoped=await page.evaluate(()=>{const calls=window.monBrowserFixture.calls;return ["register_playback_preparation","prepare_meeting_playback","cancel_playback_preparation"].map(cmd=>calls.find(c=>c.command===cmd)?.args.preparationId);});
  check(scoped.every(id=>id===scoped[0])&&typeof scoped[0]==="string"&&await count("read_playback_chunk")===0&&await count("finish_playback_preparation")===0,"native cancellation was unscoped or scheduled audio");
  results.push("running native verification receives the same reserved ID, rejects at its cancellation checkpoint and publishes no audio");

  await boot();await holdMeeting();await player.getByRole("button",{name:"再生準備を取り消す",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="cancel_playback_preparation"));await barrier();
  check(await page.evaluate(()=>window.cancelBarrierState)==="pending","cancel acknowledgement incorrectly meant worker completion");
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.pending[0].release());await settled();await released();
  check(await count("read_playback_chunk")===0&&await count("finish_playback_preparation")===0,"late cancelled worker was adopted");
  results.push("native cancellation acknowledgement alone never releases the global barrier; the actual preparation must terminate");

  await boot({losePrepare:true});await start.click();await player.getByRole("alert").waitFor();await released();
  check(await count("cancel_playback_preparation")>=1&&await count("finish_playback_preparation")===0&&await count("read_playback_chunk")===0&&await count("close_playback")===0,"lost prepare response did not close by ID or fabricated a token");
  results.push("a lost preparation reply closes its published native audio by reserved ID without guessing a playback token");

  await boot({loseReceipt:true});await start.click();await player.getByRole("alert").waitFor();await released();
  const receipt=await page.evaluate(()=>{const c=window.monBrowserFixture.calls;return {ack:c.find(c=>c.command==="finish_playback_preparation"),close:c.find(c=>c.command==="close_playback")};});
  check(receipt.ack.args.playbackToken===receipt.close.args.token&&await count("read_playback_chunk")===0,"lost receipt was apparent success or leaked known audio");
  results.push("lost receipt acknowledgement closes the known exact token even when native ownership was already transferred");

  await boot({losePrepare:true,failCancel:true});await start.click();await player.getByRole("alert").waitFor();await barrier();await settled();
  check(await page.evaluate(()=>window.cancelBarrierState)==="rejected"&&await page.evaluate(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length)===1,"failed cleanup allowed data operations or forgot native ownership");
  await page.evaluate(()=>window.monBrowserFixture.preparation.failCancel=false);await barrier();await settled();await released();
  check(await page.evaluate(()=>window.cancelBarrierState)==="fulfilled"&&await count("read_playback_chunk")===0,"explicit cleanup retry failed or played audio");
  results.push("failed cancellation retains native ownership and rejects the data barrier; explicit retry later releases it");

  await boot({loseReceipt:true,failClose:true});await start.click();await player.getByRole("alert").waitFor();await barrier();await settled();
  check(await page.evaluate(()=>window.cancelBarrierState)==="rejected"&&await page.evaluate(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length)===1,"failed known-token close was swallowed by the barrier");
  await page.evaluate(()=>window.monBrowserFixture.preparation.failClose=false);await barrier();await settled();await released();
  check(await page.evaluate(()=>window.cancelBarrierState)==="fulfilled"&&await count("read_playback_chunk")===0,"known-token retry did not release or replayed");
  results.push("known-token close failures stay in the barrier after receipt loss and are released only by a successful retry");

  await boot();await start.click();await player.getByRole("button",{name:"一時停止",exact:true}).click();
  await player.getByRole("slider",{name:"会議全体の再生位置",exact:true}).fill("960000");await player.getByRole("combobox",{name:"速度",exact:true}).selectOption("1.5");
  const summary=dock.getByText("再生トラック",{exact:true});if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.holdModes=["system"]);await mode.selectOption("system");
  await page.waitForFunction(()=>window.monBrowserFixture.meetingAudio.pending.length===1);await mode.selectOption("both");await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  const newest=await page.evaluate(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions)[0]);
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.pending[0].release());await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.preparation.reservations).length===0);
  check(await player.getByRole("slider",{name:"会議全体の再生位置",exact:true}).inputValue()==="960000"&&await player.getByRole("combobox",{name:"速度",exact:true}).inputValue()==="1.5"&&await page.evaluate(token=>!!window.monBrowserFixture.meetingAudio.sessions[token],newest),"old cancelled preparation released or replaced newer audio checkpoint");
  results.push("a late cancelled track preparation cannot replace or release newer playback, its exact position or speed");

  await boot({responsive:true});await editor.getByRole("button",{name:"編集を開始",exact:true}).click();await row.getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=row.getByRole("textbox",{name:"00:00:00の発言",exact:true});await field.fill("取消でも未確定本文を保持します。");
  await field.evaluate(e=>{window.cancelInput=e;e.setSelectionRange(2,5);});const saves=await count("save_transcript_draft");
  await page.evaluate(()=>window.monBrowserFixture.holdPlayback=true);await row.getByRole("button",{name:"音声を再生",exact:true}).click();
  const utterance=dock.getByRole("group",{name:"発言の音声再生",exact:true});await page.waitForFunction(()=>typeof window.monBrowserFixture.releasePlayback==="function");
  await utterance.getByRole("button",{name:"再生準備を取り消す",exact:true}).click();await barrier();await settled();await released();
  check(await field.inputValue()==="取消でも未確定本文を保持します。"&&await field.evaluate(e=>e===window.cancelInput&&e.selectionStart===2&&e.selectionEnd===5)&&await count("save_transcript_draft")===saves&&await count("read_playback_chunk")===0,"utterance cancellation altered input, selection, publication or audio");
  results.push("utterance preparation exposes native cancellation too, preserving unconfirmed text, selection and publication state");

  await boot();await holdMeeting();await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).getByRole("button",{name:"設定",exact:true}).click();await dock.waitFor({state:"hidden"});await barrier();
  check(await page.evaluate(()=>window.cancelBarrierState)==="pending","unmount dropped a still-running preparation from the global barrier");
  await page.evaluate(()=>window.monBrowserFixture.meetingAudio.pending[0].release());await settled();await released();
  check(await count("read_playback_chunk")===0,"detached preparation revived audio");
  results.push("unmounted playback retains pending native work in the global barrier until actual termination and release");

  await boot();await start.click();await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();
  const normal=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>["register_playback_preparation","prepare_meeting_playback","finish_playback_preparation"].includes(c.command)));
  check(normal.length===3&&normal.map(c=>c.command).join(",")==="register_playback_preparation,prepare_meeting_playback,finish_playback_preparation"&&normal.every(c=>c.args.preparationId===normal[0].args.preparationId)&&await page.evaluate(()=>Object.keys(window.monBrowserFixture.preparation.reservations).length)===0,"normal playback did not transfer exact reserved ownership before audio");
  await barrier();await settled();await released();
  results.push("normal playback registers, prepares and acknowledges the exact ID once, then transfers audio ownership to the player");
  return {passed:results.length,results};
}
