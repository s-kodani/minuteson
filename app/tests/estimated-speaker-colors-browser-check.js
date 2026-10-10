// Production review/merge controls with synthetic interval and draft IPC only.
async (page,capturePath,detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>{
    const f=window.monBrowserFixture;
    f.setSpeakers([{id:"speaker-a",label:"話者A",displayName:null},{id:"speaker-b",label:"話者B",displayName:"確認済みの名前"}]);
    f.speakerReviews={"speech-a":{estimatedState:"overlap",segmentCount:4,overlapCount:1,segments:[
      {startUs:0,endUs:2000000,speakerId:"speaker-a",confidence:null},
      {startUs:1000000,endUs:3000000,speakerId:"speaker-b",confidence:null},
      {startUs:3000000,endUs:4000000,speakerId:null,confidence:null},
      {startUs:4000000,endUs:5000000,speakerId:"unregistered",confidence:null},
    ],overlaps:[{startUs:1000000,endUs:2000000,speakerIds:["speaker-a","speaker-b"]}]}};
  });
  await page.locator(".meeting-name").first().click();
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true});
  const row=editor.locator('[data-utterance-id="speech-a"]');
  const review=row.locator(".speaker-review");await review.locator(":scope > summary").click();
  await review.getByRole("button",{name:"この区間から聞く",exact:true}).first().waitFor();
  const source=review.locator('[data-speaker-id="speaker-a"]').first(),other=review.locator('[data-speaker-id="speaker-b"]').first();
  const tone=await row.locator('.assigned-speakers [data-speaker-id="speaker-a"]').getAttribute("data-speaker-tone");
  check(await source.getAttribute("data-speaker-tone")===tone&&await source.textContent()==="話者A名前未設定"&&await review.locator('.speaker-unset').count()===2&&await review.locator('.estimated-speakers .speaker-label').count()===2,"estimated/original overlap lost ID colors, both names or unconfirmed status");
  check(await review.getByText("話者不明",{exact:false}).count()===2&&await review.locator('[data-speaker-id="unregistered"]').count()===0&&(await review.innerText()).includes("信頼度なし"),"unknown/null interval became a colored identity or invented confidence");
  results.push("estimated segments and overlap use the original IDs, both names and name-unset text; null/unregistered labels remain unknown with no fabricated confidence");
  const reads=()=>page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="get_speaker_reviews"&&c.args.request.includeIntervals).length);
  const writes=()=>page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length);
  const originalReads=await reads();await source.evaluate(e=>window.estimatedSpeakerNode=e);
  for(const theme of ["light","dark"]){
    await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);
    const colors=await source.evaluate(e=>{const s=getComputedStyle(e);return [s.color,s.backgroundColor];});
    check(await row.locator('.assigned-speakers [data-speaker-id="speaker-a"]').evaluate((e,colors)=>{const s=getComputedStyle(e);return s.color===colors[0]&&s.backgroundColor===colors[1];},colors)&&await source.evaluate(e=>e===window.estimatedSpeakerNode)&&await reads()===originalReads&&await writes()===0,"theme changed ID color consistency, review DOM, read ownership or draft");
    await review.scrollIntoViewIfNeeded();await page.screenshot({path:`estimated-speaker-${theme}.png`});
    results.push(`${theme} estimated and assigned ID colors agree without a source reread or draft write`);
  }
  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();await detailUI.openSpeakerPanel();
  await editor.getByRole("combobox",{name:"統合元",exact:true}).selectOption("speaker-a");
  await editor.getByRole("combobox",{name:"統合先",exact:true}).selectOption("speaker-b");
  await editor.getByRole("button",{name:"話者を統合",exact:true}).click();
  const confirm=page.getByRole("dialog",{name:"話者の統合を確認",exact:true});await confirm.waitFor();await confirm.getByRole("button",{name:"話者を統合する",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.draft?.payload.speaker_merges?.some(m=>m.source_id==="speaker-a"&&m.target_id==="speaker-b"));
  check(await row.locator('.assigned-speakers [data-speaker-id="speaker-a"]').count()===0&&await row.locator('.assigned-speakers [data-speaker-id="speaker-b"]').count()===1&&await source.getAttribute("data-speaker-tone")===tone&&(await source.textContent()).includes("統合前の推定ラベル")&&(await source.textContent()).includes("名前未設定")&&await reads()===originalReads,"merge changed estimated source identity or hid its origin/unconfirmed state");
  results.push("confirmed merge changes assigned rows only; estimated source keeps its original ID color and explicit pre-merge label without rereading intervals");
  const beforeNameWrites=await writes(),group=editor.getByRole("group",{name:"話者Bの名前編集",exact:true});
  await group.getByRole("textbox",{name:"話者B",exact:true}).fill("別の未確定名");
  check(await other.textContent()==="確認済みの名前"&&await writes()===beforeNameWrites,"unconfirmed name leaked into estimated labels");
  await group.getByRole("button",{name:"確定",exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('[data-utterance-id="speech-a"] .speaker-review [data-speaker-id="speaker-b"]')?.textContent==="別の未確定名");
  check(await other.getAttribute("data-speaker-tone")===await row.locator('.assigned-speakers [data-speaker-id="speaker-b"]').getAttribute("data-speaker-tone")&&await reads()===originalReads&&await writes()===beforeNameWrites+1,"confirmed same-ID rename changed tone, reloaded interval or wrote unrelated data");
  results.push("estimated labels retain confirmed names while typing; one explicit name save changes only same-ID labels without changing colors or interval ownership");
  check(await page.evaluate(()=>!window.monBrowserFixture.calls.some(c=>["start_capture","start_diarization"].includes(c.command))),"synthetic verification started capture or analysis");
  return {passed:results.length,results};
}
