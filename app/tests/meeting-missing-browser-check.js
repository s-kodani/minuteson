// Production UI with synthetic IPC, silent PCM and the real browser audio clock.
async (page,capturePath,detailUI)=>{
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const slider=player.getByRole("slider",{name:"会議全体の再生位置",exact:true});
  const mode=dock.getByRole("combobox",{name:"会議全体の再生トラック",exact:true});
  const speed=player.getByRole("combobox",{name:"速度",exact:true});
  const next=player.getByRole("button",{name:"次の保存音声へ",exact:true});
  const count=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(call=>call.command===command).length,command);
  const gap=(trackKind,startUs,endUs,reasonCode="capture_timestamp_gap")=>({trackKind,startUs,endUs,reasonCode});
  const prepare=async(gaps,endUs=60000000)=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(({gaps,endUs})=>Object.assign(window.monBrowserFixture.meetingAudio,{gaps,endUs}),{gaps,endUs});
    await page.locator(".meeting-name").first().click();
    await page.getByRole("region",{name:"文字起こし編集",exact:true}).getByRole("button",{name:"編集を開始",exact:true}).waitFor();
    await dock.getByRole("button",{name:"会議全体を再生",exact:true}).click();
    await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  };
  const details=async()=>{const summary=dock.getByText("再生トラック",{exact:true});if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();};
  await prepare([gap("system",0,100000,"capture_start_gap"),gap("mic",300000,500000),gap("system",400000,600000),gap("mic",590000,700000)]);
  check(await count("read_playback_chunk")===0&&await slider.inputValue()==="0","leading gap read or played fabricated silence");
  check((await player.innerText()).includes("欠損を無音として補いません")&&(await player.innerText()).includes("PC音声の欠損区間"),"missing input warning absent");
  results.push("leading gap is visible and paused without a native PCM read");
  await next.click();check(await slider.inputValue()==="4800"&&await count("read_playback_chunk")===0,"next saved jumped timeline or auto-played");
  await player.getByRole("button",{name:"再生",exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('input[aria-label="会議全体の再生位置"]')?.value==="14400");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  const offsets=await page.evaluate(()=>window.monBrowserFixture.calls.filter(call=>call.command==="read_playback_chunk").map(call=>call.args.offsetFrames));
  check(JSON.stringify(offsets)==="[4800]","prefetch read into or jumped past interior gap");
  check((await player.innerText()).includes("マイクの欠損区間"),"interior stop mislabeled input");
  results.push("preceding silent synthetic PCM finishes before automatic stop at the exact interior boundary");
  await next.click();check(await slider.inputValue()==="33600"&&await count("read_playback_chunk")===1,"overlapping gaps were not traversed together or auto-played");
  results.push("explicit next-saved action traverses overlapping input gaps without collapsing meeting time");
  await slider.fill("14400");await speed.selectOption("1.5");await details();await mode.selectOption("system");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  check(await slider.inputValue()==="14400"&&await speed.inputValue()==="1.5"&&await next.count()===0,"other input inherited foreign gap or lost checkpoint");
  await mode.selectOption("mic");await next.waitFor();
  check(await slider.inputValue()==="14400"&&await speed.inputValue()==="1.5"&&await count("read_playback_chunk")===1,"mode switch into gap read PCM or clamped position");
  results.push("explicit track changes preserve exact frame, speed and paused state with only selected input gaps");
  await mode.selectOption("both");await next.waitFor();
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true}),row=editor.getByRole("article",{name:"00:00:00の発言行",exact:true});
  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();await row.getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=row.getByRole("textbox",{name:"00:00:00の発言",exact:true});await field.fill("欠損を確認しても未確定の編集を保持します。");
  await field.evaluate(e=>{window.missingField=e;e.setSelectionRange(2,5);});const saves=await count("save_transcript_draft");await next.click();
  check(await field.evaluate(e=>e===window.missingField&&e.selectionStart===2&&e.selectionEnd===5)&&await count("save_transcript_draft")===saves,"gap navigation published or replaced unconfirmed editor");
  results.push("gap navigation preserves unconfirmed text DOM and selection without saving it");
  const summary=player.getByText("欠損区間を確認",{exact:true});await summary.click();
  check(await player.locator(".audio-missing li").count()===4&&(await player.innerText()).includes("0:00.300〜0:00.500"),"gap list changed microsecond source times");
  await player.getByRole("button",{name:"この欠損の後へ",exact:true}).nth(1).click();
  check(await slider.inputValue()==="33600","row action moved into an overlapping gap");
  results.push("gap list reports inputs and original times and row navigation avoids overlapping missing intervals");

  await prepare([gap("mic",0,10000000,"capture_start_gap"),gap("system",9000000,60000000,"capture_stop_gap")]);
  check(await next.isDisabled()&&(await player.innerText()).includes("この先に保存音声はありません")&&await count("read_playback_chunk")===0,"all-union gap invented available PCM");
  results.push("a union covering the remaining meeting has no fabricated next-saved destination");
  await details();await mode.selectOption("system");await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  await slider.fill("432000");await next.waitFor();check(await next.isDisabled()&&await count("read_playback_chunk")===0,"tail seek played or restarted audio");
  results.push("tail missing seek stays paused and never loops or reads PCM");

  await prepare(Array.from({length:12},(_,index)=>gap("mic",index*1000000,index*1000000+200000)));
  await player.getByText("欠損区間を確認",{exact:true}).click();check(await player.locator(".audio-missing li").count()===10,"gap page is unbounded");
  await player.getByRole("button",{name:"次の欠損一覧",exact:true}).click();check(await player.locator(".audio-missing li").count()===2,"next gap page lost entries");
  await player.getByRole("button",{name:"前の欠損一覧",exact:true}).click();
  results.push("gap list pagination keeps at most ten rows without dropping ranges");
  for(const theme of ["light","dark"])for(const width of [1280,360]) {
    await page.setViewportSize({width,height:900});await page.getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    await dock.evaluate(e=>{e.scrollTop=0;});await next.scrollIntoViewIfNeeded();
    check(await next.isVisible()&&await next.isEnabled(),"missing action hidden at narrow viewport");
    const layout=await page.evaluate(()=>{const dock=document.querySelector(".meeting-playback-dock"),warning=document.querySelector(".audio-missing"),d=dock.getBoundingClientRect(),w=warning.getBoundingClientRect();return {left:w.left,right:w.right,width:innerWidth,height:d.height,overflow:document.documentElement.scrollWidth>innerWidth};});
    check(layout.left>=0&&layout.right<=width+1&&!layout.overflow&&layout.height<=361,"gap warning overflow or oversized dock");
    await page.screenshot({path:capturePath(`missing-${theme}-${width}.png`),fullPage:true});
    results.push(`${theme} ${width}px keeps missing warning and navigation inside bounded scrollable dock`);
  }
  await page.setViewportSize({width:1200,height:900});
  await prepare([gap("mic",0,100000)]);await page.evaluate(()=>window.monBrowserFixture.meetingAudio.corrupt={missingRanges:[{trackKind:"mic",startUs:0,endUs:100000,startFrame:0,endFrame:4801,reasonCode:"dropped"}]});
  await details();await mode.selectOption("mic");await player.getByRole("alert").waitFor();
  check(await count("read_playback_chunk")===0&&(await player.innerText()).includes("欠損"),"malformed gap coordinates were accepted");
  results.push("forged missing frame coordinates close the prepared token before any PCM read");
  return {passed:results.length,results};
}
