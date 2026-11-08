// Real transcript/modal components with synthetic saved data; no user data.
async page => {
  const results=[], check=(ok,message)=>{if(!ok)throw Error(message);};
  for(const scenario of ["ordinary","filtered","filtered-top","filtered-bottom","consolidated","filtered-consolidated","filtered-uncheck-consolidated","filtered-uncheck-consolidated-lost","filtered-uncheck-unknown","filtered-retry"]){
    if(results.length)await page.reload();
    await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(scenario=>{
      const step=scenario.includes("consolidated")?2_000_000:5_000_000;
      window.monBrowserFixture.loseConsolidationReply=scenario.endsWith("-lost");
      window.monBrowserFixture.setUtterances(Array.from({length:160},(_,i)=>({
        id:`scroll-${i}`,trackId:"microphone",startUs:i*step,endUs:i*step+1_000_000,
        recognizedText:`発言${i}`,effectiveText:`発言${i}：閲覧位置を確認する合成の本文です。`,
        attributionState:scenario.startsWith("filtered-uncheck-consolidated")&&i===100?"overlap":"assigned",
        speakerIds:scenario.startsWith("filtered-uncheck-consolidated")&&i===100?["speaker-a","speaker-b"]:[i%2?"speaker-b":"speaker-a"],
      })));
    },scenario);
    await page.getByRole("button",{name:"会議A",exact:true}).click();
    const list=page.getByRole("region",{name:"発言本文",exact:true});
    await list.waitFor();
    if(scenario.startsWith("filtered"))await page.getByRole("button",{name:"田中の発言で絞り込み",exact:true}).click();
    if(scenario.includes("uncheck-consolidated"))await page.getByRole("searchbox",{name:"発言を検索",exact:true}).fill("閲覧位置");
    const row=page.locator(`[data-utterance-id="scroll-${scenario==="filtered-bottom"?158:100}"]`);
    await row.evaluate((e,scenario)=>e.scrollIntoView({block:scenario==="filtered-top"?"start":"center"}),scenario);
    if(scenario==="filtered-top")await row.evaluate(e=>{
      const list=e.closest(".utterance-list");
      list.scrollTop+=e.getBoundingClientRect().top-list.getBoundingClientRect().top+1;
    });
    const anchor=await list.evaluate((list,scenario)=>{
      const top=list.getBoundingClientRect().top;
      let row=[...list.children].find(row=>row.getBoundingClientRect().bottom>top);
      if(scenario==="filtered-top")row=row.nextElementSibling;
      return {label:row.getAttribute("aria-label"),offset:row.getBoundingClientRect().top-top,scroll:list.scrollTop};
    },scenario);
    check(anchor.scroll>1000,"fixture did not scroll into the meeting");
    await row.locator(".options-modal-trigger").click();
    const dialog=page.getByRole("dialog",{name:/の発言の話者を変更|の話者を変更/});
    await dialog.getByRole("checkbox",{name:"田中",exact:true}).uncheck();
    if(!scenario.includes("uncheck"))await dialog.getByRole("checkbox",{name:"佐藤",exact:true}).check();
    check(await row.count()===1,"unconfirmed checkbox change removed the row");
    if(scenario==="filtered-retry")await page.evaluate(()=>{window.monBrowserFixture.failSave=true;});
    await dialog.getByRole("button",{name:"保存",exact:true}).click();
    if(scenario==="filtered-retry"){
      await dialog.getByRole("alert").waitFor();
      check(await list.evaluate((list,anchor)=>Math.abs(list.scrollTop-anchor.scroll)<2,anchor),"failed assignment reset filtered scroll");
      await page.evaluate(()=>{window.monBrowserFixture.failSave=false;});
      await dialog.getByRole("button",{name:"保存を再試行",exact:true}).click();
    }
    await dialog.waitFor({state:"hidden"});
    if(scenario.endsWith("-lost"))await page.getByRole("button",{name:"発言統合を再試行",exact:true}).click();
    if(scenario.includes("consolidated"))await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===1);
    await page.waitForFunction(()=>!document.querySelector(".utterance-list")?.closest("[inert]"));
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    const position=await list.evaluate((list,anchor)=>{
      const row=[...list.children].find(row=>row.getAttribute("aria-label")===anchor.label);
      return {scroll:list.scrollTop,maxScroll:list.scrollHeight-list.clientHeight,offset:row?row.getBoundingClientRect().top-list.getBoundingClientRect().top:null};
    },anchor);
    check(position.scroll>1000 && position.offset!==null && (Math.abs(position.offset-anchor.offset)<2
      || scenario==="filtered-bottom" && Math.abs(position.scroll-position.maxScroll)<2),
      `${scenario} lost reading position: ${JSON.stringify({anchor,position})}`);
    if(scenario.startsWith("filtered")){
      check(await row.count()===0,"saved assignment still matches the old speaker filter");
      check(await page.getByRole("button",{name:"田中の発言で絞り込み",exact:true}).getAttribute("aria-pressed")==="true",
        `${scenario} reset the selected speaker filter`);
      check(await page.getByRole("button",{name:/すべての発言/}).getAttribute("aria-pressed")==="false",
        `${scenario} switched to all utterances`);
      check(await list.locator("article").count()===79,`${scenario} showed utterances from other speakers`);
      if(scenario.includes("uncheck-consolidated"))check(await page.getByRole("searchbox",{name:"発言を検索",exact:true}).inputValue()==="閲覧位置","consolidation cleared the text query");
      // MainWindow refreshes after publication; a later version notification
      // must not undo the filter that survived the consolidation reply.
      await page.evaluate(()=>window.monBrowserFixture.emitTranscriptChange());
      await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      check(await page.getByRole("button",{name:"田中の発言で絞り込み",exact:true}).getAttribute("aria-pressed")==="true",
        `${scenario} reset the filter on post-publication refresh`);
    }
    results.push(`${scenario} assignment preserves the visible transcript anchor`);
  }
  await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>window.monBrowserFixture.setUtterances([0,1,2].map(i=>({
    id:`empty-${i}`,trackId:"microphone",startUs:i*1_000_000,endUs:i*1_000_000+500_000,
    recognizedText:`発言${i}`,effectiveText:`発言${i}`,attributionState:i===1?"overlap":"assigned",
    speakerIds:i===1?["speaker-a","speaker-b"]:["speaker-b"],
  }))));
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  const filter=page.getByRole("button",{name:"田中の発言で絞り込み",exact:true});await filter.click();
  await page.locator('[data-utterance-id="empty-1"] .options-modal-trigger').click();
  const emptyDialog=page.getByRole("dialog",{name:"00:00:01の話者を変更",exact:true});
  await emptyDialog.getByRole("checkbox",{name:"田中",exact:true}).uncheck();
  await emptyDialog.getByRole("button",{name:"保存",exact:true}).click();await emptyDialog.waitFor({state:"hidden"});
  await page.waitForFunction(()=>window.monBrowserFixture.consolidationHistory?.length===1&&!document.querySelector(".utterance-list")?.closest("[inert]"));
  check(await filter.getAttribute("aria-pressed")==="true"&&await filter.innerText()==="0発言","empty result lost the selected speaker filter");
  check(await page.getByRole("region",{name:"発言本文",exact:true}).locator("article").count()===0,"empty speaker filter showed other speakers");
  check(await page.getByRole("button",{name:/すべての発言/}).getAttribute("aria-pressed")==="false","empty result selected all utterances");
  results.push("zero matching utterances keep the speaker selected after consolidation");
  return {passed:results.length,results};
}
