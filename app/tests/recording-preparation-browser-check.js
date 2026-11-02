// Production MainWindow with synthetic IPC only. Never runs the native app or capture.
async (page,capturePath,detailUI)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);},context=page.context();
  await context.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch();
    const body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const workspace=()=>page.getByRole("region",{name:"録音状態",exact:true});
  const name=()=>page.locator("#recording-title");
  const start=()=>workspace().getByRole("button",{name:"録音を開始",exact:true});
  const modal=()=>page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
  const accept=()=>modal().getByRole("button",{name:"録音を開始する",exact:true});
  const count=command=>page.evaluate(c=>window.closeFixture.calls.filter(x=>x.command===c).length,command);
  const boot=async()=>{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await name().waitFor();await page.waitForFunction(()=>!document.querySelector('#recording-title')?.disabled);
    await page.waitForFunction(()=>{const e=document.querySelector('.recording-workspace button');return e&&!e.disabled;});
    await page.waitForFunction(()=>{const e=document.querySelector('[aria-label="録音するマイク"]');return e&&!e.disabled;});
  };
  const dismissFailure=async()=>{await page.getByRole("alert").filter({hasText:"処理を完了できませんでした。"}).waitFor();await modal().getByRole("button",{name:"確認を閉じる",exact:true}).click();await modal().waitFor({state:"hidden"});};
  try {
  await boot();
  for(const [label,text] of [["新しい録音","モデル未準備でも録音できます"],["録音の会議名","日時から仮の会議名"],["録音するマイク","自動では会議アプリ"],["録音前の入力確認","毎回のテストは必須ではありません"]]){
    await workspace().getByRole("button",{name:`${label}の説明を開く`,exact:true}).click();
    const help=page.getByRole("dialog",{name:label,exact:true});await help.waitFor();
    check((await help.innerText()).includes(text),`${label}: explanation missing`);
    await page.keyboard.press("Escape");await help.waitFor({state:"hidden"});
  }
  check(await workspace().locator("label .info-trigger").count()===0,"recording info nested in input label");
  check(await page.getByRole("textbox",{name:"会議名",exact:true}).count()===1,"recording title label lost its input");
  results.push("recording and input help is adjacent and accessible without changing field labels or starting capture");
  check(/^\d{4}\/\d{2}\/\d{2} \d{2}:\d{2}の会議$/u.test(await name().inputValue())&&await start().isEnabled(),"local default title missing or required typing");
  check(await count("create_meeting")===0&&await count("start_recording")===0&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"opening preparation persisted a draft or began capture");
  results.push("new recording opens a dedicated preparation with local-time title and no DB/capture write");
  const vocabulary=()=>page.getByRole("dialog",{name:"会議の用語（任意）",exact:true});
  const openVocabulary=()=>workspace().getByRole("button",{name:"会議の用語（任意）",exact:true}).click();
  await openVocabulary();await vocabulary().getByRole("button",{name:"用語を追加",exact:true}).click();
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  check(await start().isDisabled(),"incomplete vocabulary allowed recording start");
  await openVocabulary();
  await vocabulary().getByRole("textbox",{name:"用語（必須）",exact:true}).fill("点検日");
  await vocabulary().getByRole("textbox",{name:"読み（任意）",exact:true}).fill("てんけんび");
  await vocabulary().getByRole("textbox",{name:"補足（任意・保存表示用）",exact:true}).fill("録音前の補足");
  await vocabulary().getByRole("button",{name:"用語を追加",exact:true}).click();
  const spare=vocabulary().getByRole("article",{name:"用語 2",exact:true});
  await spare.getByRole("textbox",{name:"用語（必須）",exact:true}).fill("削除する用語");
  await spare.getByRole("checkbox",{name:"有効",exact:true}).uncheck();
  await spare.getByRole("button",{name:"用語を削除",exact:true}).click();
  await page.getByRole("button",{name:"確認をキャンセル",exact:true}).click();
  check(await spare.count()===1&&!await spare.getByRole("checkbox",{name:"有効",exact:true}).isChecked(),"cancelled delete changed recording vocabulary");
  await spare.getByRole("button",{name:"用語を削除",exact:true}).click();
  await page.getByRole("button",{name:"用語を削除する",exact:true}).click();
  await spare.waitFor({state:"detached"});
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  check(await count("save_vocabulary_book")===0&&await count("create_meeting")===0&&await count("start_recording")===0,"local vocabulary was persisted before recording consent");
  check(await workspace().getByRole("button",{name:"会議の用語（任意）",exact:true}).evaluate(e=>e===document.activeElement),"vocabulary close did not restore trigger focus");
  results.push("recording vocabulary is local, incomplete input blocks start, and closing restores focus without writes");
  await name().fill("変更した点検会議");await start().click();await modal().waitFor();
  check((await modal().innerText()).includes("変更した点検会議")&&await name().isDisabled()&&await count("start_recording")===0,"confirmation target changed or began before consent");
  await page.keyboard.press("Escape");await modal().waitFor({state:"hidden"});
  check(await name().inputValue()==="変更した点検会議"&&await count("create_meeting")===0&&await count("start_recording")===0,"decline wrote or lost local title");
  results.push("confirmation locks the exact local title; declining keeps preparation without saving a meeting");
  await openVocabulary();
  check(await vocabulary().getByRole("textbox",{name:"用語（必須）",exact:true}).inputValue()==="点検日","cancelled recording confirmation lost vocabulary");
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  for(const title of ["  ","a".repeat(201)]){await name().fill(title);check(await start().isDisabled(),"invalid title allowed start");}
  await name().fill("😀".repeat(200));check(await start().isEnabled(),"Unicode scalar title was counted as UTF-16 units");
  results.push("blank and over-200 titles cannot start; 200 non-BMP characters match native title admission");
  await name().fill("二重送信しない会議");await start().click();await accept().dblclick();await modal().waitFor({state:"hidden"});
  const args=await page.evaluate(()=>window.closeFixture.calls.find(c=>c.command==="start_recording").args);
  check(args.newTitle==="二重送信しない会議"&&/^[0-9a-f-]{36}$/u.test(args.meetingId)&&args.microphoneUid===null&&args.captureScope.mode==="system"&&await count("start_recording")===1&&await count("create_meeting")===0,"new consent did not send one native start with fixed ID/title and automatic microphone");
  check(await page.evaluate(id=>window.dataFixture.meetings.filter(m=>m.id===id).length===1,args.meetingId),"new accepted recording lacks exactly one meeting");
  check(args.meetingVocabulary.length===1&&args.meetingVocabulary[0].term==="点検日"&&args.meetingVocabulary[0].reading==="てんけんび"&&args.meetingVocabulary[0].note==="録音前の補足","recording start omitted vocabulary fields");
  check(await page.evaluate(id=>window.closeFixture.vocabulary[id]?.payload[0].term==="点検日",args.meetingId),"accepted meeting did not retain its vocabulary");
  check((await workspace().innerText()).includes("録音を準備しています")&&!await name().count()&&await start().count()===0,"canonical accepted phase was replaced with idle preparation");
  results.push("double consent sends one start request; native fixture creates the confirmed ID and the same central panel reports canonical preparation");

  await page.evaluate(async()=>{
    const f=window.closeFixture,m=window.dataFixture.meetings.find(m=>m.id===f.recording.meetingId);m.lifecycle="recorded";m.sourceType="recording";
    await f.recordChanged({phase:"idle",meetingId:null,sessionId:null});
  });
  await page.getByRole("button",{name:"会議の用語",exact:true}).click();
  check(await vocabulary().getByRole("textbox",{name:"用語（必須）",exact:true}).inputValue()==="点検日","recording vocabulary missing from transcript detail");
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  results.push("recording completion opens the same saved vocabulary in transcript detail");

  await boot();await openVocabulary();await vocabulary().getByRole("button",{name:"用語を追加",exact:true}).click();
  await vocabulary().getByRole("textbox",{name:"用語（必須）",exact:true}).fill("保存失敗の用語");
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  await page.evaluate(()=>{window.closeFixture.failVocabulary=true;});
  await start().click();await accept().click();await dismissFailure();
  check(await count("start_recording")===1&&await page.evaluate(()=>window.dataFixture.meetings.length===1&&window.closeFixture.recording.phase==="idle"),"vocabulary failure left a meeting or started capture");
  await openVocabulary();check(await vocabulary().getByRole("textbox",{name:"用語（必須）",exact:true}).inputValue()==="保存失敗の用語","failed vocabulary write lost local input");
  await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  results.push("vocabulary write failure retains preparation without creating a meeting or starting capture");

  await boot();await page.evaluate(()=>{window.closeFixture.failFlush=true;});await start().click();await accept().click();await dismissFailure();
  check(await name().count()===1&&await count("start_recording")===0&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"failed edit save persisted the new draft");
  results.push("failed edit flushing retains local title and prevents both meeting creation and capture");

  await boot();await page.evaluate(()=>{window.closeFixture.startLost=true;});await start().click();await accept().click();await dismissFailure();
  check(await count("start_recording")===1&&await name().isDisabled()&&await start().count()===0,"lost accepted reply replayed or enabled new start");
  await workspace().getByRole("button",{name:"録音の受付結果を確認",exact:true}).click();
  await name().waitFor({state:"detached"});
  check(await count("start_recording")===1&&(await workspace().innerText()).includes("録音を準備しています"),"result read replayed start or fabricated idle");
  results.push("lost accepted reply freezes its local request; explicit result read adopts only the known meeting without starting again");

  await boot();await page.evaluate(()=>{window.closeFixture.startBeforeCommitFailure=true;});await start().click();await accept().click();await dismissFailure();
  await workspace().getByRole("button",{name:"録音の受付結果を確認",exact:true}).click();
  check(await start().isDisabled()&&await name().isDisabled()&&await count("start_recording")===1&&await page.evaluate(()=>window.dataFixture.meetings.length===1),"missing result was treated as permission to replay");
  const unknown=page.locator('.toast-host').getByRole('alert').filter({hasText:'保存済みの会議は確認できません'});await unknown.waitFor();
  check((await unknown.innerText()).includes("新しく準備"),"unknown result omitted safe next action in its notification");
  results.push("a missing meeting after uncertain start remains unknown and never auto-retries the same creation/start");

  await boot();await page.evaluate(()=>{window.closeFixture.failCaptureInputs=true;});await start().click();
  await page.getByRole("alert").filter({hasText:"模擬マイク一覧取得失敗"}).waitFor();
  check(await count("start_recording")===0&&await count("create_meeting")===0,"fresh microphone failure saved new meeting");
  results.push("fresh input admission fails before confirmation or any new meeting write");

  await boot();await page.keyboard.press("Meta+n");await name().waitFor();
  check(await count("create_meeting")===0&&await count("start_recording")===0,"shortcut persisted preparation");
  results.push("Cmd N follows the same dedicated preparation path without creating an empty meeting");

  for(const width of [1280,960,360])for(const theme of ["light","dark"]){
    await page.setViewportSize({width,height:640});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);
    const bounds=await workspace().evaluate(e=>{const r=e.getBoundingClientRect(),p=e.parentElement.getBoundingClientRect(),css=getComputedStyle(e.parentElement);return {width:r.width,left:r.left,right:r.right,parentLeft:p.left+parseFloat(css.paddingLeft),parentRight:p.left+e.parentElement.clientWidth-parseFloat(css.paddingRight),overflow:e.scrollWidth>e.clientWidth+1,pageOverflow:document.documentElement.scrollWidth>innerWidth};});
    check(Math.abs(bounds.left-bounds.parentLeft)<1&&Math.abs(bounds.right-bounds.parentRight)<1&&!bounds.overflow&&!bounds.pageOverflow,`preparation does not fill shared content width: ${JSON.stringify(bounds)}`);
    await start().scrollIntoViewIfNeeded();check(await start().isVisible(),"primary start hidden behind workspace tools");
    await page.screenshot({path:`recording-preparation-${theme}-${width}.png`});
    await workspace().getByRole("heading",{name:"新しい録音",exact:true}).scrollIntoViewIfNeeded();
    await page.screenshot({path:`recording-preparation-${theme}-${width}-top.png`});
    await openVocabulary();await vocabulary().waitFor();
    check(await vocabulary().evaluate(e=>e.scrollWidth<=e.clientWidth+1),"recording vocabulary dialog overflows");
    await page.screenshot({path:`recording-vocabulary-${theme}-${width}.png`});
    await page.keyboard.press("Escape");await vocabulary().waitFor({state:"hidden"});
  }
  results.push("preparation fills shared content width and primary start fits both themes at 1280/960/360px without nested tools clipping");
  await page.locator(".screen-navigation").getByRole("button",{name:"会議一覧",exact:true}).click();
  check(await page.locator('.nav-item[aria-current="page"]').getAttribute("aria-label")==="会議一覧"&&await count("start_recording")===0,"back did not return to list or began capture");
  results.push("back returns to meeting list without creating an unused draft or capture");
  return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} completed cases (${results.at(-1)??"setup"}): ${cause}; body=${await page.locator("body").innerText()}`);}
}
