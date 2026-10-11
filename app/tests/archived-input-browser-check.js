// CLI-first production MainWindow regression, synthetic closed recordings only.
async page=>{
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const card=(input="マイク")=>page.locator(".toast-card").filter({hasText:new RegExp(`終了した録音.*${input}`,"u")});
  const calls=()=>page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="acknowledge_archived_recording_input_notice").map(c=>c.args.request));
  const release=action=>page.evaluate(action=>{const state=window.closeFixture.archived;state.hold[action]=false;const replies=state.replies.filter(r=>r.action===action);state.replies=state.replies.filter(r=>r.action!==action);replies.forEach(r=>r.resolve());},action);
  const reload=async()=>{await page.evaluate(()=>window.closeFixture.persistArchivedFixture());await page.reload();await page.waitForFunction(()=>!!window.closeFixture);};
  const seed=options=>page.evaluate(options=>{const f=window.closeFixture;f.archived.rows=[f.newArchivedRow(options)];return f.recordChanged({phase:"idle"});},options);
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.evaluate(()=>{const f=window.closeFixture;f.archived.hold.receive=true;const d=document.createElement("dialog");d.id="archive-cover";d.textContent="合成表示遮蔽";document.body.append(d);d.showModal();});
    await seed({both:true,loss:2,issue:"missing",title:"終了した点検会議"});
    await page.waitForFunction(()=>window.closeFixture.archived.replies.length===2);await page.waitForTimeout(1100);
    check(await card().count()===0&&(await calls()).every(c=>c.action==="receive"),"held receipt invented rendering");
    check(await page.locator('[data-input-kind]').count()===0,"closed history became current input badge");
    results.push("closed recordings claim independent microphone/system events without inventing current input state or rendering");
    await release("receive");await card().waitFor({state:"attached"});await page.waitForTimeout(350);
    check((await calls()).every(c=>c.action==="receive"),"modal-covered archive toast was rendered");
    await page.evaluate(()=>{document.querySelector(".mon-notification-stack").style.opacity="0";document.querySelector("#archive-cover").close();});await page.waitForTimeout(350);
    check((await calls()).every(c=>c.action==="receive"),"transparent archive toast was rendered");
    await page.evaluate(()=>{const host=document.querySelector(".mon-notification-stack");host.style.opacity="1";host.style.transform="translateY(-2000px)";});await page.waitForTimeout(350);
    check((await calls()).every(c=>c.action==="receive"),"off-viewport archive toast was rendered");
    results.push("modal coverage, transparency and off-viewport cards cannot acknowledge archived rendering");
    await page.evaluate(()=>{document.querySelector(".mon-notification-stack").style.transform="";document.querySelector("#archive-cover").showModal();window.closeFixture.showQueueBlockers();document.querySelector("#archive-cover").close();});
    await page.waitForTimeout(400);check(await card().count()===0&&(await calls()).every(c=>c.action==="receive"),"queued archive card fabricated render");
    const old=(await calls())[0];await reload();await card().waitFor();
    await page.waitForFunction(()=>Object.values(window.closeFixture.archived.rows[0].lanes).every(l=>l[0].delivery.rendered));
    check(await page.evaluate(old=>{const row=window.closeFixture.archived.rows[0],n=row.lanes[old.noticeId.includes(":system:")?"system":"microphone"][0];return n.delivery.generation===2&&n.recipientId!==old.recipientId;},old),"unshown historical head not generation fenced");
    check(await page.evaluate(async old=>{try{await window.closeFixture.archivedAction({...old,action:"render",receiveId:null});return false;}catch{return true;}},old),"old archived render accepted after replacement");
    results.push("queued unshown notices survive WebView replacement under new generations; old recipient is rejected");
    await card().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await card().filter({hasText:"復帰していました"}).waitFor();
    check(await page.evaluate(()=>window.closeFixture.archived.rows[0].lanes.system[0].delivery.sequence)===1,"closing microphone advanced system lane");
    await page.waitForFunction(()=>window.closeFixture.archived.rows[0].lanes.microphone[0].delivery.rendered);
    results.push("confirmed interruption closes before recovery and each historical input advances independently");
    await card().getByRole("button",{name:"会議を開く",exact:true}).click();await page.getByRole("tabpanel",{name:"文字起こし",exact:true}).waitFor();
    check(await page.locator(".topbar").innerText().then(text=>text.includes("会議詳細")),"archive action did not open meeting detail");
    const details=page.locator(".archived-input-notices details");await details.locator("summary").click();
    check((await details.innerText()).includes("保持不足 2件")&&(await details.innerText()).includes("取得履歴がありません"),"retained diagnostics lost after navigation/dismissal");
    check(await page.locator(".archived-input-notices").evaluate(e=>e.getBoundingClientRect().height<=innerHeight*.16+1),"historical diagnostics consumed the editor's workspace");
    results.push("meeting action navigates through production UI without playback; missing history and loss remain after dismissal");
    for(const input of ["マイク","PC音声"]){const n=card(input);if(await n.count())await n.getByRole("button",{name:input==="マイク"?"情報通知を閉じる":"注意通知を閉じる",exact:true}).click();}
    await card("PC音声").filter({hasText:"復帰していました"}).waitFor();await page.waitForFunction(()=>window.closeFixture.archived.rows[0].lanes.system[0].delivery.rendered);
    await reload();await page.waitForTimeout(700);check(await card().count()===0&&await card("PC音声").count()===0,"rendered past event replayed after reload");
    check(await page.locator(".archived-input-notices summary").count()===1,"restarting lost diagnostic state");
    results.push("render-confirmed events do not replay in replacement WebView while diagnostics persist");

    await page.evaluate(()=>{const f=window.closeFixture;f.archived.lost.render=true;f.archived.lost.receive=true;});await seed({title:"応答喪失の会議"});
    await card().waitFor();await page.waitForFunction(()=>window.closeFixture.archived.rows[0].lanes.microphone[0].delivery.rendered);
    await card().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await card().filter({hasText:"復帰していました"}).waitFor();
    check(await page.evaluate(()=>Object.values(window.closeFixture.archived.rows[0].closed).length)===1,"lost rendered receipt could not reconcile closure");
    results.push("lost receive/render replies are reconciled by exact native readback, not local completion assumptions");
    await page.evaluate(()=>{window.closeFixture.archived.holdRead=true;});await page.waitForFunction(()=>window.closeFixture.archived.reads.length===1);
    const before=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_archived_recording_input_notices").length);
    await page.waitForTimeout(1800);check(await card().count()===0,"stale archived read kept presentation");
    check(await page.getByLabel("終了済み録音の通知読込待ちの要対応状態").count()===1,"hung page read did not expose its waiting state");
    check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_archived_recording_input_notices").length)===before,"hung archive read caused overlapping request");
    await page.evaluate(()=>{const state=window.closeFixture.archived;state.holdRead=false;state.reads.splice(0).forEach(resolve=>resolve());state.failRead=true;});
    await page.getByLabel("終了済み録音の通知読込の要対応状態").waitFor();check(await card().count()===0,"late old page resurrected expired display");
    results.push("hung page reads remain single-flight, stale delivery disappears, and failed reads expose retained warning");
    await page.evaluate(()=>{const f=window.closeFixture;f.archived.failRead=false;f.archived.lost.render=false;f.archived.lost.receive=false;f.archived.hold.receive=true;});await seed({title:"受付保留の会議"});
    await page.waitForFunction(()=>window.closeFixture.archived.replies.length===1);await page.waitForTimeout(1200);
    check(await page.evaluate(()=>window.closeFixture.archived.replies.length)===1,"hung receive duplicated request");
    await page.evaluate(()=>{window.closeFixture.archived.failRead=true;});await page.getByLabel("終了済み録音の通知読込の要対応状態").waitFor();
    await release("receive");check(await card().count()===0,"failed source resurrected via old receipt");
    await page.evaluate(()=>{window.closeFixture.archived.failRead=false;});await card().waitFor();
    results.push("read failure preserves pending acknowledgement slots, with no duplicate hung receives or optimistic late display");
    await page.evaluate(()=>window.dataFixture.changed({kind:"backup",phase:"writing",busy:true}));await card().waitFor({state:"hidden"});
    const held=await calls();await page.waitForTimeout(1100);check((await calls()).length===held.length,"maintenance continued archive writes");
    await page.evaluate(()=>window.dataFixture.changed({kind:null,phase:"idle",busy:false}));await card().waitFor();
    results.push("known maintenance suppresses delivery and writes until a fresh post-maintenance read");

    await page.evaluate(()=>{const f=window.closeFixture,uid=i=>`${i.toString(16).padStart(8,"0")}-aaaa-4aaa-8aaa-aaaaaaaaaaaa`;
      f.archived.rows=Array.from({length:40},(_,i)=>f.newArchivedRow({sessionId:uid(i+1),title:`旧形式${i+1}`,legacy:true}));f.archived.rows.push(f.newArchivedRow({sessionId:uid(50),title:"二ページ目の会議",loss:3}));
    });await page.waitForFunction(()=>window.closeFixture.archived.rows.at(-1).lanes.microphone[0].delivery.rendered);
    await card().filter({hasText:"二ページ目の会議"}).waitFor();
    check(await page.evaluate(()=>window.closeFixture.calls.some(c=>c.command==="get_archived_recording_input_notices"&&c.args.afterSessionId!==null)),"archive never traversed next page");
    results.push("32-row cursor traversal reaches later recordings, and legacy rows do not create notifications");
    await page.evaluate(()=>{const f=window.closeFixture;f.archived.rows.unshift(f.newArchivedRow({sessionId:"00000000-aaaa-4aaa-8aaa-aaaaaaaaaaaa",title:"後から終了した会議"}));});
    await page.waitForFunction(()=>window.closeFixture.archived.rows[0].lanes.microphone[0].delivery.rendered);
    results.push("cyclic traversal discovers newly closed random UUIDs behind the prior cursor");
    await seed({title:"復元された点検会議",both:true,loss:2,historySaveIssue:"failed",deliveryUnavailable:true});
    await page.waitForFunction(()=>document.querySelector(".archived-input-notices details")?.textContent.includes("復元した通知の受付を確認できません"));
    // The previous source can remain until the bounded cursor traversal wraps
    // and completes an authoritative new cycle. Wait for its normal removal;
    // never dismiss a card or delete a receipt to make this check pass.
    await card().waitFor({state:"hidden"});await card("PC音声").waitFor({state:"hidden"});
    const imported=page.locator(".archived-input-notices details");
    if(!await imported.evaluate(e=>e.open))await imported.locator(":scope > summary").click();
    let importedText=await imported.innerText();
    check(importedText.includes("取得履歴の書込みを確定できません")&&importedText.includes("マイク通知の保持不足 2件")&&importedText.includes("PC音声通知の保持不足 2件"),"imported diagnosis/loss did not reach production review UI");
    check(!importedText.includes("通知履歴を読めません"),"readable imported diagnosis was labelled unreadable");
    const importedOwner=await page.evaluate(()=>window.closeFixture.archived.rows[0].sessionId);
    check((await calls()).every(c=>c.sessionId!==importedOwner)&&await card().count()===0&&await card("PC音声").count()===0,"immutable imported state started reception or rendering");
    results.push("imported diagnosis reaches production review UI without pretending that pending delivery or current capture is available");
    await reload();await page.waitForFunction(()=>document.querySelector(".archived-input-notices details")?.textContent.includes("復元した通知の受付を確認できません"));
    const reopened=page.locator(".archived-input-notices details");if(!await reopened.evaluate(e=>e.open))await reopened.locator(":scope > summary").click();
    importedText=await reopened.innerText();
    check(importedText.includes("書込みを確定できません")&&importedText.includes("保持不足 2件")&&(await calls()).every(c=>c.sessionId!==importedOwner),"reload removed immutable import issues or adopted receipt authority");
    results.push("imported failed persistence and retained loss survive WebView replacement without acknowledgement requests");
    for(const theme of ["light","dark"]){await page.emulateMedia({colorScheme:theme});for(const width of [960,360]){
      await page.setViewportSize({width,height:900});await page.waitForTimeout(200);
      check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`imported diagnosis UI overflows ${width} ${theme}`);
      await page.screenshot({path:`imported-${theme}-${width}.png`});
    }}
    results.push("readable imported diagnostics retain bounded 960/360px light/dark layouts");
    await page.setViewportSize({width:1200,height:900});
    const visualTitle="終了済み録音の通知を確認するための点検会議";
    await seed({title:visualTitle,both:true,loss:4,issue:"invalid"});await card().filter({hasText:visualTitle}).waitFor();
    await page.waitForFunction(()=>Object.values(window.closeFixture.archived.rows[0].lanes).every(l=>l[0]?.delivery.rendered));
    const visualDetails=page.locator(".archived-input-notices details");
    if(!await visualDetails.evaluate(e=>e.open))await visualDetails.locator(":scope > summary").click();
    for(const theme of ["light","dark"]){await page.emulateMedia({colorScheme:theme});for(const width of [960,360]){
      await page.setViewportSize({width,height:900});await page.waitForTimeout(200);
      check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`archive UI overflows ${width} ${theme}`);
      check(await card().evaluate(e=>{const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight;}),"archive card outside viewport");
      await page.screenshot({path:`archive-${theme}-${width}.png`});
    }}
    check((await page.locator(".archived-input-notices details").innerText()).includes("現在の機器接続・取得・保存の状態ではありません"),"diagnostics imply present capture");
    results.push("960/360px light/dark layouts retain historical warnings and distinguish them from present acquisition");
    const forbidden=["start_capture_test","start_recording","stop_recording","prepare_meeting_playback","prepare_playback","prepare_capture_test_playback"];
    check(await page.evaluate(forbidden=>window.closeFixture.calls.every(c=>!forbidden.includes(c.command)),forbidden),"archive workflow requested recording or playback");
    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.waitForTimeout(700);
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>c.command!=="get_archived_recording_input_notices")),"mini window polled closed-recording delivery");
    results.push("archive workflow never requests capture/playback, and mini window does not own archived delivery");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}
  finally{await page.context().unrouteAll();await page.evaluate(()=>sessionStorage.removeItem("MON_ARCHIVED_INPUT_FIXTURE")).catch(()=>{});}
}
