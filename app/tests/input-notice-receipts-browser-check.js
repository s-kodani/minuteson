// Disposable WebViews share a synthetic backend receipt ledger only when seeded.
// No native process, capture, device access or audio output is involved.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');await route.fulfill({response,body});
  });
  const state=async value=>page.waitForFunction(v=>document.querySelector('[data-input-kind="microphone"]')?.dataset.inputState===v,value);
  const toast=()=>page.locator(".toast-card").filter({hasText:/マイクの取得|同じマイク/u});
  const reception=async value=>page.waitForFunction(v=>document.querySelector('[data-input-kind="microphone"]')?.dataset.noticeReception===v,value);
  const calls=command=>page.evaluate(command=>window.closeFixture.calls.filter(c=>c.command===command).length,command);
  const set=async(counter,state="disconnected",target="main",both=false)=>page.evaluate(([counter,state,target,both])=>{
    const f=window.closeFixture,id=f.recording.sessionId,kind=state==="connected"?"recovered":"interrupted",binding={state,sourceEpoch:state==="connected"?counter+1:counter,interruptions:counter,reason:state==="connected"?null:"device_disconnected"};
    const n=input=>({id:`${id}:${input}:${counter}:${kind}`,input,target,kind,interruptions:counter,recipientId:null});
    f.inputStatus={schemaVersion:3,ownerId:id,revision:(f.inputStatus?.revision??0)+1,reportAgeMs:0,microphone:binding,system:both?{...binding,reason:state==="connected"?null:"stream_stopped"}:{state:"connected",sourceEpoch:1,interruptions:0,reason:null},notices:both?[n("microphone"),n("system")]:[n("microphone")]};
  },[counter,state,target,both]);
  const release=()=>page.evaluate(()=>{const f=window.closeFixture;f.receiptHoldBefore=false;f.receiptHoldAfter=false;f.receiptPending.splice(0).forEach(resolve=>resolve());});
  const reload=async()=>{await page.evaluate(()=>window.closeFixture.persistInputReceiptFixture());await page.reload();await page.waitForFunction(()=>!!window.closeFixture);if(!new URL(page.url()).searchParams.has("window"))await page.getByRole("button",{name:"録音画面へ",exact:true}).click();};
  try{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.getByRole("button",{name:"録音を開始",exact:true}).click();const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.evaluate(()=>{const f=window.closeFixture;f.refreshInputStatus=true;f.receiptHoldBefore=true;return f.recordChanged({phase:"recording"});});
    await set(1);await state("disconnected");await page.waitForFunction(()=>window.closeFixture.receiptPending.length===1);
    const reads=await calls("get_recording_input_status"),receives=await calls("receive_recording_input_notice");await reception("unconfirmed");
    check(await toast().count()===0,"notice rendered without receipt");check((await page.locator('[data-input-kind="microphone"]').innerText()).includes("通知確認待ち"),"pending receipt hidden");
    check(await calls("get_recording_input_status")>reads,"receipt blocked status polling");check(await calls("receive_recording_input_notice")===receives,"hung receipt overlapped same event");results.push("hung receipt has no optimistic toast or duplicate request; live binding polls continue and pending result is explicit");
    await page.evaluate(()=>window.closeFixture.failInputStatus=true);await state("unknown");await page.evaluate(()=>window.closeFixture.failInputStatus=false);await state("disconnected");await page.waitForTimeout(1100);
    check(await calls("receive_recording_input_notice")===receives,"temporary failed read overlapped pending receipt");check(await toast().count()===0,"readback fabricated acceptance");results.push("transient failed status read cannot overlap an unresolved receipt for the same event");
    await release();await toast().waitFor();await reception("received");const first=await page.evaluate(()=>Object.values(window.closeFixture.inputReceipts)[0].recipientId);
    await page.evaluate(()=>{window.closeFixture.failRecordingSnapshot=true;window.dispatchEvent(new Event("focus"));});await toast().waitFor({state:"hidden"});
    await page.evaluate(()=>{window.closeFixture.failRecordingSnapshot=false;window.dispatchEvent(new Event("focus"));});await state("disconnected");await reception("received");check(await toast().count()===0,"failed session bridge reopened accepted receiver");results.push("recreating the receipt controller after a session bridge failure does not redeliver accepted events");
    await reload();await state("disconnected");await reception("received");check(await toast().count()===0,"new WebView replayed accepted event");check(await calls("receive_recording_input_notice")===0,"new recipient claimed old event");results.push("accepted receipt survives WebView recreation and suppresses delivery to its new recipient");
    await page.evaluate(()=>window.closeFixture.receiptBeforeFailure=true);await set(2);await reception("unconfirmed");check(await toast().count()===0,"failed acceptance rendered notice");
    const failed=await calls("receive_recording_input_notice");await page.waitForTimeout(250);check(await calls("receive_recording_input_notice")<=failed+1,"retry storm");
    await reload();await toast().waitFor();await reception("received");check(await page.evaluate(first=>Object.values(window.closeFixture.inputReceipts).some(r=>r.recipientId!==first),first),"new WebView never accepted unreceived event");results.push("unaccepted event remains deliverable after recreation; failed receipt is not treated as success");
    await toast().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();
    await page.evaluate(()=>{const f=window.closeFixture;f.receiptLost=true;f.receiptHoldAfter=true;});await set(3);await state("disconnected");await page.waitForFunction(()=>window.closeFixture.receiptPending.length===1);
    await toast().waitFor();await reception("received");check(await page.evaluate(()=>Object.values(window.closeFixture.inputReceipts).length)===3,"lost response caused duplicate commit");
    await release();await page.waitForTimeout(200);check(await toast().count()===1,"lost response duplicated or erased reconciled notification");results.push("fresh authoritative status reconciles acceptance while its original response remains hung or is lost");
    await toast().getByRole("button",{name:"注意通知を閉じる",exact:true}).click();await reload();await reception("received");check(await toast().count()===0,"lost-response acceptance replayed after reload");results.push("lost-response receipt also survives WebView recreation");
    await page.evaluate(()=>window.closeFixture.receiptHoldBefore=true);await set(4);await page.waitForFunction(()=>window.closeFixture.receiptPending.length===1);
    await page.evaluate(()=>{const f=window.closeFixture;f.inputStatus=null;return f.recordChanged({sessionId:crypto.randomUUID()});});await state("unknown");await release();await page.waitForTimeout(250);
    check(await toast().count()===0,"old owner receipt resurrected notice");check(await page.evaluate(()=>Object.values(window.closeFixture.inputReceipts).length)===3,"old owner mutated receipts");results.push("late old-owner receipt cannot claim or display an event in the replacement recording");
    await page.evaluate(()=>window.closeFixture.receiptHoldBefore=true);await set(1);await page.waitForFunction(()=>window.closeFixture.receiptPending.length===1);
    await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));await state("unknown");await release();await page.waitForTimeout(250);check(await toast().count()===0,"stop resurrected notice");results.push("stopping invalidates pending acceptance without a late notification");
    await page.goto("http://127.0.0.1:1420/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);await page.setViewportSize({width:360,height:116});
    await page.evaluate(()=>{const f=window.closeFixture;f.refreshInputStatus=true;f.receiptHoldBefore=true;return f.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()});});
    await set(1,"disconnected","recorder",true);await state("disconnected");await page.waitForTimeout(650);
    check(await calls("receive_recording_input_notice")===0&&await page.evaluate(()=>Object.values(window.closeFixture.inputReceipts).length)===0,"mini accepted unseen notices");
    check(await page.locator(".mini-input-notice,.mini-input-receipt-pending").count()===0,"removed mini notification UI returned");
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth+1),"pending two-input state overflow");await page.screenshot({path:"input-receipt-mini-pending-360x116.png"});results.push("both unconfirmed receipts fit the native mini geometry without claiming saving");
    await release();await reload();await state("disconnected");check(await calls("receive_recording_input_notice")===0&&await page.evaluate(()=>Object.values(window.closeFixture.inputReceipts).length)===0,"mini recreation acknowledged unseen notices");results.push("both input events remain unreceived across mini WebView recreation");
    await set(1,"connected","recorder",true);await state("connected");check(await calls("receive_recording_input_notice")===0,"mini received recovery without presenting it");
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1),"recovery overflow");await page.screenshot({path:"input-receipt-mini-recovery-360x116.png"});results.push("compact recovery telemetry fits without consuming either recovery event");
    check(await page.evaluate(()=>window.closeFixture.calls.every(c=>!["start_capture_test","start_recording","stop_recording"].includes(c.command))),"mini issued capture");results.push("mini receipt checks never start or stop native capture");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}finally{await page.context().unrouteAll();await page.evaluate(()=>sessionStorage.removeItem("MON_INPUT_RECEIPT_FIXTURE")).catch(()=>{});}
}
