// Production UI with synthetic IPC. No capture device or real meeting access.
async (page) => {
  const results=[], check=(value,message)=>{if(!value)throw Error(message);};
  const out="output/playwright/initial-speaker-count";
  const imported=()=>page.locator("dialog[open]").filter({has:page.getByRole("heading",{name:"音声を取り込む",exact:true})});
  const importCount=()=>imported().getByRole("combobox",{name:"会議全体の話者数",exact:true});
  const openImport=async()=>{
    await page.goto("http://127.0.0.1:1420/tests/import-browser.html");
    await page.getByRole("button",{name:"音声を取り込む",exact:true}).click();
    await page.waitForFunction(()=>window.monImportFixture?.import.phase==="ready");
    await importCount().waitFor();
  };
  await openImport();
  check(await importCount().inputValue()==="","new import should start automatic");
  await importCount().selectOption("5");
  await page.keyboard.press("Escape");
  const discard=page.getByRole("dialog",{name:"取り込み確認内容の破棄を確認",exact:true});
  await discard.waitFor();
  await page.keyboard.press("Escape");
  await discard.waitFor({state:"hidden"});
  check(await importCount().inputValue()==="5","declining discard lost the count");
  results.push("import count is editable before processing and survives declined discard");
  await page.setViewportSize({width:960,height:800});
  await page.screenshot({path:`${out}/import-5.png`});
  await page.evaluate(()=>{window.monImportFixture.saveDelayMs=1000;});
  await imported().getByRole("button",{name:"取り込む",exact:true}).click();
  await page.waitForFunction(()=>window.monImportFixture.calls.some(c=>c.command==="start_audio_import"));
  const request=await page.evaluate(()=>window.monImportFixture.calls.find(c=>c.command==="start_audio_import").args.request);
  check(request.options.numSpeakers===5,"initial import did not submit five speakers");
  check(await importCount().isDisabled(),"count editable after import admission");
  await imported().waitFor({state:"hidden"});
  results.push("five speakers sent with initial import and locked during save");
  await openImport();
  check(await importCount().inputValue()==="","import count leaked into a new file");
  results.push("new import resets to automatic");

  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const workspace=()=>page.getByRole("region",{name:"録音状態",exact:true});
  const bootRecording=async()=>{
    await page.goto("http://127.0.0.1:1420/");
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.waitForFunction(()=>!document.querySelector("#recording-title")?.disabled);
  };
  await bootRecording();
  const confirmation=()=>page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
  const count=()=>confirmation().getByRole("combobox",{name:"会議全体の話者数",exact:true});
  check(await workspace().getByRole("combobox",{name:"会議全体の話者数",exact:true}).count()===0,"speaker selection remains in preparation");
  await workspace().getByRole("button",{name:"録音を開始",exact:true}).click();
  await confirmation().waitFor();
  check(await count().inputValue()==="","new recording must start automatic");
  await count().selectOption("5");
  for(const width of [1280,360]) {
    await page.setViewportSize({width,height:900});
    await count().scrollIntoViewIfNeeded();
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),`overflow at ${width}`);
    await page.screenshot({path:`${out}/recording-${width}.png`});
  }
  check(await confirmation().locator(".confirmation-impact").count()===0,"simple recording confirmation retained prose");
  await page.keyboard.press("Escape");await confirmation().waitFor({state:"hidden"});
  check(await page.evaluate(()=>!window.closeFixture.calls.some(c=>c.command==="start_recording")),"cancel started recording");
  await workspace().getByRole("button",{name:"録音を開始",exact:true}).click();
  check(await count().inputValue()==="5","cancelled confirmation reset recording count");
  await confirmation().getByRole("button",{name:"録音を開始する",exact:true}).click();
  await confirmation().waitFor({state:"hidden"});
  const calls=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="start_recording"));
  check(calls.length===1&&calls[0].args.numSpeakers===5&&!('speakerCounts' in calls[0].args),"recording request lost whole-meeting count or replayed");
  results.push("start confirmation alone selects the whole-meeting count, survives cancellation, fits narrow screens and sends five exactly once");
  await bootRecording();
  await page.evaluate(()=>{window.closeFixture.startLost=true;});
  await workspace().getByRole("button",{name:"録音を開始",exact:true}).click();
  check(await count().inputValue()==="","recording count leaked into new preparation");
  await count().selectOption("2");
  await confirmation().getByRole("button",{name:"録音を開始する",exact:true}).click();
  await confirmation().getByRole("alert").waitFor();
  await confirmation().getByRole("button",{name:"確認を閉じる",exact:true}).click();
  await confirmation().waitFor({state:"hidden"});
  check(await page.locator(".recording-workspace .speaker-count-fields select:not(:disabled)").count()===0,"lost start reply unlocked frozen counts");
  const lost=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="start_recording"));
  check(lost.length===1&&lost[0].args.numSpeakers===2,"lost reply replayed or changed count");
  results.push("new recording resets counts; lost start response keeps chosen counts frozen without replay");
  return {passed:results.length,results};
}
