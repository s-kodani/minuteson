// Production UI with synthetic IPC and silent PCM. No device/OS capture.
async page=>{
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  await page.goto("http://127.0.0.1:1420/");
  await page.waitForFunction(()=>!!window.closeFixture);
  await page.getByRole("button",{name:"新しい録音",exact:true}).click();
  let panel=page.getByRole("region",{name:"録音前の5秒テスト",exact:true});
  const start=panel.getByRole("button",{name:"5秒録音して確認",exact:true});
  await start.waitFor();await page.waitForFunction(()=>[...document.querySelectorAll('.capture-test-panel button')].some(b=>b.textContent==="5秒録音して確認"&&!b.disabled));
  check(await panel.locator(":scope > button").count()===1,"idle test area must only offer the five-second test");
  check(await panel.getByRole("button",{name:"テストの結果を再確認",exact:true}).count()===0,"removed refresh button is still visible");
  await start.click();
  const modal=page.getByRole("dialog",{name:"5秒の録音前テストを開始しますか？",exact:true});
  await modal.getByRole("button",{name:"5秒テストを開始する",exact:true}).click();
  await modal.waitFor({state:"hidden"});
  panel=page.getByRole("dialog",{name:"録音前の入力確認",exact:true});
  await page.evaluate(()=>{const f=window.closeFixture;f.testChanged({phase:"ready",busy:false,
    microphone:null,system:{frames:240000,sampleRate:48000,channels:2,durationMs:5000,hasGaps:false}});});
  await panel.getByText("PC音声の検証が完了しました（マイク音声は未受信）",{exact:true}).waitFor();
  const mic=panel.getByRole("button",{name:"マイクのテスト音声を再生",exact:true});
  const pc=panel.getByRole("button",{name:"PCのテスト音声を再生",exact:true});
  check(await mic.isDisabled()&&await pc.isEnabled(),"partial result did not preserve mic/disable missing PC");
  check((await panel.innerText()).includes("マイク音声は未受信です")&&await panel.locator('.action-error').count()===0,"absence displayed as red capture failure");
  check(await page.evaluate(()=>window.closeFixture.calls.every(c=>c.command!=="prepare_capture_test_playback"&&c.command!=="start_recording")),"partial result autoplayed or started recording");
  await pc.click();await panel.getByRole("button",{name:"一時停止",exact:true}).waitFor();
  check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="prepare_capture_test_playback").map(c=>c.args.trackKind).join(',')==="system"),"missing PC was prepared as playable audio");
  await panel.getByRole("button",{name:"テスト音声の再生を閉じる",exact:true}).click();
  await page.waitForFunction(()=>Object.keys(window.closeFixture.testPlayback.tokens).length===0);
  for(const theme of ["light","dark"]){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await mic.scrollIntoViewIfNeeded();
    check(await panel.evaluate(e=>e.scrollWidth<=e.clientWidth+1),"partial-result text overflows");
    check(await panel.evaluate(e=>{const buttons=[...e.querySelectorAll('.common-modal-body > button')].map(b=>b.getBoundingClientRect());return buttons.slice(1).every((b,i)=>b.top-buttons[i].bottom>=8);}),"test actions have no spacing");
    await page.screenshot({path:`output/playwright/test-controls-pc-only-${theme}-20261012.png`});
  }
  await panel.getByRole("button",{name:"テスト音声を破棄",exact:true}).click();
  await page.waitForFunction(()=>window.closeFixture.captureTest===null);
  return {passed:8,results:["one idle action and no manual refresh", "PC-only ready result", "microphone absence is informational", "no autoplay/native recording", "PC-only prepared playback", "both themes without overflow", "spaced test actions", "owned partial result disposed"]};
}
