async page => {
  const check=(ok,message)=>{if(!ok)throw Error(message);}, results=[];
  const calls=command=>page.evaluate(command=>window.canonicalPlaybackFixture.calls.filter(c=>c.command===command),command);
  const player=page.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const slider=player.getByRole("slider");
  for(const [label,offset,max] of [["全体",0,16000],["発言",12000,16000],["分割発言",12000,16000],["議事録の開始位置",12000,16000],["区間だけ",0,799]]) {
    const before=(await calls("read_playback_chunk")).length;
    await page.getByRole("button",{name:label,exact:true}).click();
    await page.waitForFunction(before=>window.canonicalPlaybackFixture.calls.filter(c=>c.command==="read_playback_chunk").length>before,before);
    const reads=await calls("read_playback_chunk");
    check(reads[before].args.offsetFrames===offset,`${label}: wrong first frame`);
    check(await slider.getAttribute("max")===String(max),`${label}: wrong playback length`);
    await player.getByRole("button",{name:"再生",exact:true}).waitFor();
    check(await slider.inputValue()===String(max),`${label}: did not reach EOF`);
    check(await page.getByRole("alert").count()===0,`${label}: playback error`);
    results.push(`${label}: native 16 kHz response plays from the expected offset through EOF`);
  }
  await page.getByRole("button",{name:"全体",exact:true}).click();
  await player.getByRole("button",{name:"一時停止",exact:true}).click();
  await slider.fill("8000");
  await player.getByRole("combobox",{name:"速度",exact:true}).selectOption("1.5");
  const before=(await calls("read_playback_chunk")).length;
  await player.getByRole("button",{name:"再生",exact:true}).click();
  await page.waitForFunction(before=>window.canonicalPlaybackFixture.calls.filter(c=>c.command==="read_playback_chunk").length>before,before);
  check((await calls("read_playback_chunk"))[before].args.offsetFrames===8000,"seek/resume used the wrong rate");
  await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  results.push("seek, speed change, resume and EOF retain the 16 kHz position");
  await page.evaluate(()=>window.canonicalPlaybackFixture.rate=48000);
  const rejectedBefore=(await calls("read_playback_chunk")).length;
  await page.getByRole("button",{name:"全体",exact:true}).click();
  await page.getByRole("alert").filter({hasText:"会議全体の再生情報が保存音声と一致しません"}).waitFor();
  check((await calls("read_playback_chunk")).length===rejectedBefore,"obsolete format reached PCM playback");
  await page.waitForFunction(()=>window.canonicalPlaybackFixture.sessions.size===0);
  results.push("obsolete 48 kHz response is rejected and released before PCM reads");
  return {passed:results.length,results};
}
