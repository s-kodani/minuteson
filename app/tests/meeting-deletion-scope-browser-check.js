// Local synthetic IPC only: no production files, recordings or settings.
async (page, capturePath) => {
  const check=(condition,message)=>{if(!condition)throw Error(message);};
  const reads=()=>page.evaluate(()=>window.deletionFixture.availabilityReads);
  const scenario=async name=>{
    await page.goto(`http://127.0.0.1:1420/tests/meeting-deletion-browser.html?scoped=${name}`);
    await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
  };
  const menu=async title=>{
    await page.getByRole("button",{name:`${title}の操作`,exact:true}).click();
    const remove=page.getByRole("menuitem",{name:`${title}を削除`,exact:true});
    await remove.waitFor(); return remove;
  };
  const allow=async remove=>{
    await page.waitForFunction(label=>{const element=document.querySelector(`[role="menuitem"][aria-label="${label}"]`); return element&&!element.disabled;},await remove.getAttribute("aria-label"));
  };
  const confirm=async()=>{
    await page.getByRole("heading",{name:"会議を削除",exact:true}).waitFor();
    const remove=page.getByRole("button",{name:"削除",exact:true});
    await page.waitForFunction(()=>!document.querySelector('.deletion-dialog .stop-button')?.disabled);
    await remove.click();
  };
  await scenario("stale");
  await page.clock.runFor(3200);
  check((await reads()).length===0,"closed meeting list polled deletion availability");
  const [target,other]=await page.evaluate(()=>window.dataFixture.meetings.map(meeting=>meeting.id));
  const trigger=page.getByRole("button",{name:"既存会議の操作",exact:true});
  await trigger.focus(); await page.keyboard.press("Enter");
  let remove=page.getByRole("menuitem",{name:"既存会議を削除",exact:true});
  await allow(remove);
  const openReads=await reads();
  check(openReads.length>0&&openReads.every(ids=>ids.length===1&&ids[0]===target),"menu queried outside its own meeting");
  await page.evaluate(id=>window.deletionFixture.updateMeetingBlocker(id,true),other);
  await page.clock.runFor(1500);
  check((await reads()).length===openReads.length,"unrelated job refreshed the open menu");
  await page.evaluate(id=>window.deletionFixture.updateMeetingBlocker(id,true),target);
  await page.getByRole("menuitem",{name:"ジョブ一覧で確認",exact:true}).waitFor();
  check(await remove.isDisabled(),"started job did not disable the open menu");
  await page.evaluate(id=>window.deletionFixture.updateMeetingBlocker(id,false),target);
  await allow(remove);
  await page.evaluate(id=>window.deletionFixture.updateMeetingBlocker(id,false),other);
  await page.keyboard.press("Escape");
  const closedReads=(await reads()).length;
  await page.clock.runFor(1500);
  check((await reads()).length===closedReads&&await trigger.getAttribute("aria-expanded")==="false","closed menu kept checking");

  await page.getByText("音声保存エラー",{exact:true}).waitFor();
  remove=await menu("既存会議"); await allow(remove);
  await page.screenshot({path:capturePath("meeting-deletion-scope-enabled.png")});
  await remove.click(); await confirm();
  await page.getByRole("button",{name:"既存会議",exact:true}).waitFor({state:"hidden"});
  check(await page.getByRole("button",{name:"残す会議",exact:true}).count()===1,"deletion removed another meeting");
  remove=await menu("残す会議"); await allow(remove); await remove.click(); await confirm();
  await page.getByRole("button",{name:"残す会議",exact:true}).waitFor({state:"hidden"});

  await scenario("active");
  remove=await menu("既存会議");
  check(await remove.isDisabled(),"running target was deletable");
  await page.getByRole("menuitem",{name:"ジョブ一覧で確認",exact:true}).waitFor();
  await page.screenshot({path:capturePath("meeting-deletion-scope-blocked.png")});
  await page.keyboard.press("Escape");
  const unrelated=await menu("残す会議"); await allow(unrelated);
  await page.keyboard.press("Escape");
  await page.getByRole("button",{name:"合成ジョブ停止・削除可否応答を復旧",exact:true}).click();
  remove=await menu("既存会議"); await allow(remove);

  await scenario("failure"); remove=await menu("既存会議");
  check(await remove.isDisabled(),"unverified state enabled deletion");
  await page.locator(".meeting-row-menu-note:visible").filter({hasText:/削除可否を確認できません/}).waitFor();
  await page.evaluate(()=>{window.dataFixture.meetingAvailabilityFail=false;});
  await page.getByRole("menuitem",{name:"削除可否を再確認",exact:true}).click();
  await allow(remove);

  await scenario("review-race"); remove=await menu("既存会議"); await allow(remove); await remove.click();
  await page.getByText(/確認の直前に対象会議の処理が始まりました/).first().waitFor();
  check(!/確認の受付を確定できません/.test(await page.locator("body").innerText()),"known rejection became an unknown request");
  remove=await menu("既存会議"); await allow(remove); await remove.click();
  await page.getByRole("button",{name:"キャンセル",exact:true}).click();

  await scenario("execute-race"); remove=await menu("既存会議"); await allow(remove); await remove.click(); await confirm();
  await page.getByRole("alert").filter({hasText:/実行の直前に対象会議の処理が始まりました/}).first().waitFor();
  check(await page.locator(".meeting-name").filter({hasText:"既存会議"}).count()===1,"rejected request deleted its target");
  await page.getByRole("button",{name:"削除",exact:true}).click();
  await page.getByRole("button",{name:"既存会議",exact:true}).waitFor({state:"hidden"});
  return {passed:9,results:["idle list stays quiet and the keyboard-opened menu reads only its target and its job state","stale recording deletion","recorded deletion","running target blocked with job navigation","unrelated meeting deletion available","stopped target becomes deletable","read failure and in-menu recovery","review rejection can retry","execution rejection can retry"]};
}
