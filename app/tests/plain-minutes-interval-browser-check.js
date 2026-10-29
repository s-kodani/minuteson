// Production UI with synthetic storage/PCM. No recording or real model runs.
async page=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  let stage="initial-preview";
  try {
  const count=cmd=>page.evaluate(cmd=>window.__mon.intervalCalls.filter(c=>c.cmd===cmd).length,cmd);
  const panel=()=>page.getByRole("region",{name:"議事録",exact:true});
  const link=()=>panel().getByRole("button",{name:"この区間の開始時刻から再生: 00:00:01.000001–00:00:03.999999",exact:true});
  const player=()=>page.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const close=async()=>{
    await page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true}).click();await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();await player().waitFor({state:"hidden"});
    await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();await page.getByRole("tab",{name:"議事録",exact:true}).click();await link().waitFor();
  };
  await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();await page.getByRole("tab",{name:"議事録",exact:true}).click();await link().waitFor();
  check(await count("prepare_meeting_playback")===0&&await count("read_playback_chunk")===0,"mount prepared or played audio");
  check(await panel().locator(".markdown-audio-link").count()===1&&await panel().locator("a[href],img").count()===0,"invalid/remote links became active");
  const dockHeight=await page.locator('.meeting-playback-dock').evaluate(el=>el.getBoundingClientRect().height);
  results.push("saved Markdown previews one valid interval without auto-play or executable/remote links");
  stage="first-interval";
  await link().click();await player().getByRole("button",{name:"一時停止",exact:true}).waitFor();
  check(await page.locator('.meeting-playback-dock .audio-player').count()===1,"interval created duplicate/outside dock player");
  check(Math.abs(await page.locator('.meeting-playback-dock').evaluate(el=>el.getBoundingClientRect().height)-dockHeight)<=1,"interval notification changed dock height");
  check(await page.evaluate(()=>{const r=window.__mon.intervalCalls.find(c=>c.cmd==="prepare_meeting_playback").args.request;return r.meetingId==="m1"&&r.startReference.startUs===1000001&&r.startReference.endUs===3999999&&r.startReference.expectedDurationUs===60000000&&r.mode==="mixed";}),"native request rounded or changed interval/mode");
  await player().getByRole("button",{name:"一時停止",exact:true}).click();
  await page.screenshot({path:"plain-minutes-interval.png"});await close();
  results.push("click starts at the exact fractional time on the whole timeline in the one shared dock; pause and navigation release work");
  for(const [modes,expected] of [[['mixed'],'mixed']]){
    stage=`audio-mode-${expected}`;
    await page.evaluate(modes=>{window.__mon.audioSummary.modes=modes;window.__mon.emitPlain("minutes-version-changed");},modes);
    await page.waitForFunction(modes=>JSON.stringify(window.__mon.audioSummary.modes)===JSON.stringify(modes),modes);
    // Re-enter to reload the shared track settings from the new saved source.
    await close();
    await link().click();await player().getByRole("button",{name:"一時停止",exact:true}).waitFor();
    check(await page.evaluate(expected=>window.__mon.intervalCalls.filter(c=>c.cmd==="prepare_meeting_playback").at(-1).args.request.mode===expected,expected),"available audio mode ignored");
    await close();
  }
  results.push("recorded canonical meeting audio uses the same native mode without a model");
  stage="no-audio-edit";
  const played=await count("prepare_meeting_playback");
  await page.evaluate(()=>{window.__mon.audioSummary.modes=[];window.__mon.emitPlain("minutes-version-changed");});await link().waitFor({state:"hidden"});
  check(await panel().getByText("点検区間",{exact:true}).isVisible()&&await count("prepare_meeting_playback")===played,"no-audio text disappeared or played");
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();
  const input=()=>panel().getByRole("textbox",{name:"議事録のMarkdown本文",exact:true});await input().fill("# 音声なしでも人間が編集\n");
  check(await input().inputValue()==="# 音声なしでも人間が編集\n"&&await page.evaluate(()=>!window.__mon.plainCalls.some(c=>c.cmd==="save_minutes_markdown")),"audio availability affected human input or autosaved");
  await panel().getByRole("button",{name:"議事録のプレビューを表示",exact:true}).click();
  // Return to the fixture's saved body without a write, using explicit discard.
  await page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true}).click();
  await page.locator("dialog[open]").getByRole("button",{name:"編集を破棄して続ける",exact:true}).click();
  await page.evaluate(()=>window.__mon.audioSummary.modes=['mixed']);await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();await page.getByRole("tab",{name:"議事録",exact:true}).click();await link().waitFor();
  results.push("no audio leaves interval text visible and never blocks or autosaves human Markdown editing");
  stage="wrong-interval";
  await page.evaluate(()=>window.__mon.intervalMode="wrong");const readBefore=await count("read_playback_chunk");await link().click();
  await page.getByRole("alert").filter({hasText:"会議全体の再生情報"}).waitFor();
  check(await count("read_playback_chunk")===readBefore,"mismatched interval played PCM");await player().getByRole("button",{name:"再生",exact:true}).waitFor();
  await page.evaluate(()=>window.__mon.intervalMode="success");await link().click();await player().getByRole("button",{name:"一時停止",exact:true}).waitFor();await close();
  results.push("wrong native interval is rejected and closed before reading PCM; explicit retry succeeds");
  stage="missing-interval";
  await page.evaluate(()=>window.__mon.intervalMode="gap");const gapReads=await count("read_playback_chunk");await link().click();
  await page.getByRole("alert").filter({hasText:"欠損補完済みの連続音声"}).waitFor();
  check(await count("read_playback_chunk")===gapReads,"legacy sparse output read PCM");
  check(await player().getByRole("button",{name:"音声欠損を確認",exact:true}).count()===0,"removed gap dialog remains");await close();
  results.push("legacy sparse output is rejected before PCM reads; canonical playback requires continuous audio");
  stage="cancel-held-interval";
  await page.evaluate(()=>window.__mon.intervalMode="held");const closedBefore=await count("close_playback");await link().click();await player().getByRole("button",{name:"再生準備を取り消す",exact:true}).waitFor();
  const countBefore=await count("read_playback_chunk");await player().getByRole("button",{name:"再生準備を取り消す",exact:true}).click();await player().getByRole("slider",{name:"会議全体の再生位置",exact:true}).waitFor();
  await page.evaluate(()=>window.__mon.intervalRelease());await page.waitForFunction(before=>window.__mon.intervalCalls.filter(c=>c.cmd==="close_playback").length>before,closedBefore);
  check(await count("read_playback_chunk")===countBefore&&await player().getByRole("slider",{name:"会議全体の再生位置",exact:true}).isDisabled(),"late cancelled result played or revived controls");
  results.push("cancelling a pending verification releases its late native token without reviving playback");
  await page.evaluate(()=>window.__mon.intervalMode="success");await link().click();await player().getByRole("button",{name:"一時停止",exact:true}).waitFor();
  await page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true}).click();await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
  check(await player().count()===0,"navigation retained interval controls");
  results.push("navigation releases interval ownership and removes the shared controls");
  return results;
  } catch(cause) {
    throw Error(`${stage}: ${String(cause)}; calls=${JSON.stringify(await page.evaluate(()=>window.__mon.intervalCalls))}; UI=${await page.locator('.meeting-playback-dock').innerText()}`);
  }
}
