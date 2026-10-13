// Production controls and BrowserAudioClock; synthetic IPC serves only silent PCM.
async (page,capturePath,detailUI)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const slider=player.getByRole("slider",{name:"会議全体の再生位置",exact:true});
  const gap=(trackKind,startUs,endUs)=>({trackKind,startUs,endUs,reasonCode:"capture_timestamp_gap"});
  const prepare=async(gaps,endUs=1002855)=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(({gaps,endUs})=>Object.assign(window.monBrowserFixture.meetingAudio,{gaps,endUs}),{gaps,endUs});
    await page.locator(".meeting-name").first().click();await player.waitFor();
    check(await slider.inputValue()==="0","idle cursor is not at zero");
    check(await dock.getByRole("button",{name:"再生設定",exact:true}).count()===1&&await dock.getByRole("button",{name:"音声欠損を確認",exact:true}).count()===0,"duplicate/removed menu remains");
  };
  const play=()=>player.getByRole("button",{name:"再生",exact:true}).click();
  const finish=async()=>{
    await page.waitForFunction(()=>{const e=document.querySelector('input[aria-label="会議全体の再生位置"]');return e&&e.value===e.max&&e.max!=="1";});
    await player.getByRole("button",{name:"再生",exact:true}).waitFor();
    check(await slider.getAttribute("step")==="1"&&await slider.inputValue()===await slider.getAttribute("max"),"slider cannot represent exact final frame");
    const reads=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="read_playback_chunk").map(c=>c.args.offsetFrames));
    check(reads.length>0,"play from zero did not read saved audio");return reads;
  };
  await prepare([gap("system",0,100000),gap("mic",300000,400000),gap("system",350000,500000),gap("mic",900000,1002855)]);
  await play();const reads=await finish();
  check(JSON.stringify(reads)==="[4800,24000]","read entered a leading/interior/tail gap or skipped valid PCM");
  results.push("play at zero skips leading and overlapping gaps, finishes saved PCM, and reaches exact end across a trailing gap");
  await play();await finish();
  results.push("play after completion replays saved audio without manual slider adjustment");
  await prepare([]);await play();await finish();
  results.push("ordinary non-step-aligned audio ends at the range maximum");
  await prepare([gap("mic",0,1002855)]);await play();
  await page.waitForFunction(()=>{const e=document.querySelector('input[aria-label="会議全体の再生位置"]');return e&&e.value===e.max&&e.max!=="1";});
  check(await page.evaluate(()=>window.monBrowserFixture.calls.every(c=>c.command!=="read_playback_chunk")),"all-missing audio invented PCM");
  results.push("all-missing audio never reads PCM or loops");
  for(const theme of ["light","dark"])for(const width of [1280,360]){
    await page.setViewportSize({width,height:900});await detailUI.applyPlaybackTheme(theme);
    check(await dock.evaluate(e=>e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth),"dock overflows");
    await page.screenshot({path:capturePath(`playback-${theme}-${width}.png`)});
  }
  results.push("both themes and desktop/narrow widths keep a single menu and bounded playback dock");
  return {passed:results.length,results};
}
