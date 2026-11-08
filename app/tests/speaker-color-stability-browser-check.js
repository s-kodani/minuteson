// Persisted lineage keys are synthetic; all labels/editor/modal components are real.
async (page, _capturePath, detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.evaluate(()=>window.monBrowserFixture.setUtterances(Array.from({length:160},(_,i)=>({
    id:`color-${i}`,trackId:"microphone",startUs:i*2_000_000,endUs:i*2_000_000+1_000_000,
    recognizedText:`発言${i}`,effectiveText:`色の保持を確認する発言${i}`,attributionState:"assigned",speakerIds:[i%2?"speaker-b":"speaker-a"],
  }))));
  const openMeeting=()=>page.getByRole("button",{name:"会議A",exact:true}).click();
  const settle=()=>page.waitForFunction(()=>!!document.querySelector(".speaker-grid .speaker-label")&&!document.querySelector(".utterance-list")?.closest("[inert]"));
  const snapshot=()=>page.evaluate(async()=>{
    const {speakerTone,speakerPalette}=await import("/src/speaker-colors.ts");
    const analysis=await window.__TAURI_INTERNALS__.invoke("get_current_analysis",{meetingId:"meeting-0"});
    const tones={},ids={},byId={},bad=[];
    const dark=document.documentElement.dataset.theme==="dark"||!document.documentElement.dataset.theme&&matchMedia("(prefers-color-scheme: dark)").matches;
    const rgb=hex=>`rgb(${[1,3,5].map(i=>parseInt(hex.slice(i,i+2),16)).join(", ")})`;
    const visibleColor=(label,tone)=>{
      const style=getComputedStyle(label.querySelector(".speaker-avatar")),palette=speakerPalette[tone];
      if(style.color!==rgb(palette[dark?2:0])||style.backgroundColor!==rgb(palette[dark?3:1]))bad.push(`wrong visible avatar colors for ${label.dataset.speakerId}`);
    };
    for(const label of document.querySelectorAll(".speaker-grid .speaker-label")){
      const id=label.dataset.speakerId,key=analysis.speakerColorKeys[id]??id,tone=Number(label.dataset.speakerTone);
      if(tone!==speakerTone("meeting-0",key))bad.push(`wrong lineage tone for ${id}`);
      visibleColor(label,tone);
      tones[key]=tone;ids[key]=id;byId[id]=tone;
    }
    for(const label of document.querySelectorAll(".utterance-list .speaker-label")){
      if(Number(label.dataset.speakerTone)!==byId[label.dataset.speakerId])bad.push(`row/sidebar mismatch for ${label.dataset.speakerId}`);
      visibleColor(label,Number(label.dataset.speakerTone));
    }
    return {tones,ids,bad};
  });
  await openMeeting();await settle();
  const initial=await snapshot();let expected={...initial.tones};
  const verify=async name=>{
    await settle();const current=await snapshot();
    check(current.bad.length===0,`${name}: ${current.bad.join(",")}`);
    check(JSON.stringify(Object.entries(current.tones).sort())===JSON.stringify(Object.entries(expected).sort()),`${name}: speaker colors changed`);
    results.push(name);return current;
  };
  await verify("initial sidebar and utterance colors use native lineage keys");
  await page.getByRole("button",{name:"田中の発言で絞り込み",exact:true}).click();
  const row=page.locator('[data-utterance-id="color-100"]');
  await row.locator(".options-modal-trigger").click();
  const assignment=page.getByRole("dialog",{name:/の話者を変更/});
  await assignment.getByRole("checkbox",{name:"田中",exact:true}).uncheck();
  await assignment.getByRole("checkbox",{name:"佐藤",exact:true}).check();
  await verify("checkbox changes do not change colors before saving");
  await assignment.getByRole("button",{name:"保存",exact:true}).click();await assignment.waitFor({state:"hidden"});
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===1);
  const consolidated=await verify("automatic consolidation keeps hues despite fresh speaker UUIDs");
  check(consolidated.ids["lineage:speaker-a"]!==initial.ids["lineage:speaker-a"],"test did not remap speaker UUIDs");
  await page.evaluate(()=>window.monBrowserFixture.emitTranscriptChange());
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  await verify("a fresh analysis response keeps colors");
  await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).getByRole("button",{name:"会議一覧",exact:true}).click();
  await openMeeting();await verify("reopening the editor obtains the same persisted keys");
  for(const theme of ["light","dark"]){await detailUI.applyPlaybackTheme(theme);await verify(`${theme} theme retains palette indexes`);}
  await page.getByRole("button",{name:"田中の名前を変更",exact:true}).click();
  const rename=page.getByRole("dialog",{name:"話者名を変更",exact:true});
  await rename.getByRole("textbox",{name:"表示名",exact:true}).fill("佐藤");
  await rename.getByRole("button",{name:"保存",exact:true}).click();await rename.waitFor({state:"hidden"});
  await verify("renaming to another speaker's display name does not change hues");
  await page.getByRole("button",{name:"話者を追加",exact:true}).click();
  const add=page.getByRole("dialog",{name:"新しい話者",exact:true});
  await add.getByRole("textbox").fill("佐藤");
  await add.getByRole("button",{name:"作成して保存",exact:true}).click();await add.waitFor({state:"hidden"});
  const manual=await snapshot(),manualKey=Object.keys(manual.tones).find(key=>key.startsWith("manual:"));
  check(!!manualKey,"manual speaker was not created");expected[manualKey]=manual.tones[manualKey];
  await verify("manual speakers use their durable manual ID before publication");
  const ids=await page.locator(".utterance-row").evaluateAll(rows=>rows.slice(20,22).map(row=>row.dataset.utteranceId));
  for(let i=0;i<ids.length;i++){
    await page.locator(`[data-utterance-id="${ids[i]}"] .options-modal-trigger`).click();
    for(const box of await assignment.getByRole("checkbox").all())await box.uncheck();
    await assignment.getByRole("checkbox").nth(2).check();
    if(i===1)await page.evaluate(()=>{window.monBrowserFixture.loseConsolidationReply=true;});
    await assignment.getByRole("button",{name:"保存",exact:true}).click();await assignment.waitFor({state:"hidden"});
  }
  await page.getByRole("button",{name:"発言統合を再試行",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===2);
  const published=await verify("manual speaker colors survive consolidation and lost-reply retry");
  check(published.ids[manualKey]!==manual.ids[manualKey],"manual speaker UUID did not change");
  await page.getByRole("button",{name:"話者を統合",exact:true}).click();
  const merge=page.getByRole("dialog",{name:"話者の統合",exact:true});
  await merge.getByRole("combobox",{name:"統合元",exact:true}).selectOption(published.ids[manualKey]);
  await merge.getByRole("combobox",{name:"統合先",exact:true}).selectOption(published.ids["lineage:speaker-b"]);
  await merge.getByRole("button",{name:"統合して保存",exact:true}).click();await merge.waitFor({state:"hidden"});
  delete expected[manualKey];
  await page.waitForFunction(()=>document.querySelectorAll(".speaker-grid .speaker-label").length===2);
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===3);
  await verify("speaker merging retains the target speaker's color");
  return {passed:results.length,results};
}
