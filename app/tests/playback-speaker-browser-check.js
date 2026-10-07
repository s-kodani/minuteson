// Production React controls, synthetic IPC and silent AudioContext only.
// Explicit timeline injections below isolate consumer guards; not real audio QA.
async (page, capturePath, detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const editor=page.getByRole("region",{name:"文字起こし編集",exact:true});
  const dock=page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const player=dock.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const mode=dock.getByRole("combobox",{name:"会議全体の再生トラック",exact:true});
  const slider=player.getByRole("slider",{name:"会議全体の再生位置",exact:true});
  const row=id=>editor.locator(`[data-utterance-id="${id}"]`);
  const count=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command).length,command);
  const boot=async({fail=false,imported=false,palette=false}={})=>{
    await page.setViewportSize({width:1280,height:900});
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(async({fail,imported,palette})=>{
      const fixture=window.monBrowserFixture;
      fixture.failTracks=fail;
      fixture.trackPlan={tracks:imported?[{id:"native-import",kind:"imported",startUs:0,endUs:15000000}]:[
        {id:"native-mic",kind:"mic",startUs:0,endUs:15000000},
        {id:"native-system",kind:"system",startUs:0,endUs:15000000},
      ]};
      if(imported)fixture.setMeetingSource("meeting-0","audio_import");
      const utterance=(id,trackId,speakerIds)=>({id,trackId,startUs:0,endUs:15000000,recognizedText:`${id} 点検日を確認します。`,effectiveText:`${id} 点検日を確認します。`,attributionState:speakerIds.length>1?"overlap":"assigned",speakerIds});
      fixture.setUtterances(imported?[
        utterance("imported","native-import",["speaker-a"]),utterance("unknown","microphone",["speaker-b"]),
      ]:[utterance("mic","native-mic",["speaker-a"]),utterance("system","native-system",["speaker-b"]),
        utterance("overlap","native-mic",["speaker-a","speaker-b"]),utterance("unknown","microphone",["speaker-a"])]);
      if(palette){
        const {speakerTone}=await import("/src/speaker-colors.ts"),speakers=[],tones=new Set();
        for(let i=0;tones.size<12&&i<1000;i++){
          const id=`palette-${i}`,tone=speakerTone("meeting-0",id);
          if(!tones.has(tone)){tones.add(tone);speakers.push({id,label:`話者${tone+1}`,displayName:tone===11?null:`参加者${tone+1}`});}
        }
        if(tones.size!==12)throw Error("fixture could not cover palette");
        fixture.setSpeakers(speakers);
        fixture.setUtterances(speakers.map((speaker,i)=>utterance(`palette-${i}`,"native-mic",[speaker.id])));
      }
    },{fail,imported,palette});
    await page.getByRole("button",{name:"開く",exact:true}).first().click();
    await editor.getByRole("button",{name:"編集を開始",exact:true}).waitFor();
    const summary=dock.getByText("再生トラック",{exact:true});
    if(!await summary.evaluate(e=>e.parentElement.open))await summary.click();
    if(!fail)await page.waitForFunction(()=>window.monBrowserFixture.calls.filter(c=>c.command==="get_diarization_tracks").length>=2);
  };
  const play=async()=>{await dock.getByRole("button",{name:"会議全体を再生",exact:true}).click();await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();};
  const pause=async()=>{await player.getByRole("button",{name:"一時停止",exact:true}).click();await player.getByRole("button",{name:"再生",exact:true}).waitFor();};
  const expectPlaying=async ids=>{
    await page.waitForFunction(ids=>JSON.stringify(Array.from(document.querySelectorAll('.utterance-row[aria-current="true"]'),e=>e.dataset.utteranceId).sort())===JSON.stringify([...ids].sort()),ids);
    const actual=await editor.locator('.utterance-playing').evaluateAll(elements=>elements.map(e=>({id:e.dataset.utteranceId,current:e.getAttribute("aria-current"),label:e.querySelector('.utterance-playing-label')?.textContent})));
    check(actual.length===ids.length&&actual.every(e=>ids.includes(e.id)&&e.current==="true"&&e.label==="再生中"),"playback class, current and text disagree");
  };
  const syntheticPosition=async(timeUs,patch={})=>page.evaluate(async({timeUs,patch})=>{
    const {playbackTimeline}=await import("/src/playback-follow.ts"),current=playbackTimeline.snapshot();
    if(!current)throw Error("missing production timeline");
    playbackTimeline.publish({...current,timeUs,playing:true,...patch});
  },{timeUs,patch});

  await boot();await play();await expectPlaying(["mic","system","overlap"]);
  check(!await dock.getByRole("checkbox",{name:"再生位置に追従",exact:true}).isChecked(),"follow default unexpectedly on");
  results.push("actual silent both-track playback highlights all concurrent mapped rows even with following OFF; ID-looking unknown tracks are never guessed");
  await mode.selectOption("mic");await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();await expectPlaying(["mic","overlap"]);
  results.push("actual microphone switch excludes simultaneous PC rows without identifying a person from track origin");
  await mode.selectOption("system");await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();await expectPlaying(["system"]);
  results.push("actual PC switch highlights only mapped PC speech while retaining the shared meeting position");
  await pause();await expectPlaying([]);await slider.fill("960000");await player.getByRole("button",{name:"再生",exact:true}).click();await expectPlaying([]);
  results.push("paused playback has no current speech; actual silent seek into a gap never picks the nearest row");
  await pause();await slider.fill("0");await player.getByRole("button",{name:"再生",exact:true}).click();await expectPlaying(["system"]);
  await page.evaluate(async()=>{const {stopAllPlayback}=await import("/src/playback-barrier.ts");await stopAllPlayback();});await expectPlaying([]);
  results.push("global production playback release clears current speech instead of retaining the old owner");

  await boot();await play();await pause();
  await editor.getByRole("button",{name:"編集を開始",exact:true}).click();
  await row("mic").getByRole("button",{name:"00:00:00の発言を編集",exact:true}).click();
  const field=row("mic").getByRole("textbox",{name:"00:00:00の発言",exact:true});
  await field.fill("未確定本文と選択範囲はトラック切替で保持します。");
  await field.evaluate(e=>{window.speakerPlaybackField=e;e.setSelectionRange(2,6);});
  const saves=await count("save_transcript_draft");
  await mode.selectOption("mic");await player.getByRole("button",{name:"再生",exact:true}).waitFor();
  await syntheticPosition(1000000);await expectPlaying(["mic","overlap"]);
  await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption("dark");
  await page.waitForFunction(()=>document.documentElement.dataset.theme==="dark");
  check(await field.inputValue()==="未確定本文と選択範囲はトラック切替で保持します。"&&await field.evaluate(e=>e===window.speakerPlaybackField&&e.selectionStart===2&&e.selectionEnd===6)&&await count("save_transcript_draft")===saves,"highlight/theme lost field or published unconfirmed text");
  results.push("synthetic current-row updates remain visible while editing without remounting the field, changing selection or publishing its buffer");

  await boot();await editor.getByRole("button",{name:"編集を開始",exact:true}).click();await detailUI.openSpeakerPanel();
  const speakerLabel=row("mic").locator('.assigned-speakers [data-speaker-id="speaker-a"]');
  const tone=await speakerLabel.getAttribute("data-speaker-tone");
  const names=editor.getByRole("group",{name:"話者Aの名前編集",exact:true});
  await names.getByRole("textbox",{name:"話者A",exact:true}).fill("名前を変更しても同じ色");
  check(await speakerLabel.textContent()==="田中"&&await speakerLabel.getAttribute("data-speaker-tone")===tone,"unconfirmed name changed stable label");
  await names.getByRole("button",{name:"確定",exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('[data-utterance-id="mic"] .assigned-speakers')?.textContent.includes("名前を変更しても同じ色"));
  check(await speakerLabel.getAttribute("data-speaker-tone")===tone,"confirmed rename changed tone");
  await editor.getByRole("combobox",{name:"話者で絞り込み",exact:true}).selectOption("speaker-a");
  check(await speakerLabel.getAttribute("data-speaker-tone")===tone&&await row("system").count()===0&&await row("overlap").locator(".assigned-speakers .speaker-label").count()===2,"filter or overlap lost ID tones");
  results.push("unconfirmed and confirmed renames, speaker filtering and overlap labels retain colors by stable speaker ID, not name or display order");

  await boot({fail:true});await play();await expectPlaying([]);
  const retry=editor.getByRole("button",{name:"発言のトラック対応を再読込",exact:true});await retry.waitFor();
  const prepares=await count("prepare_meeting_playback"),writes=await count("save_transcript_draft"),reads=await count("get_diarization_tracks");
  await page.evaluate(()=>window.monBrowserFixture.failTracks=false);await retry.click();await expectPlaying(["mic","system","overlap"]);
  check(await count("get_diarization_tracks")===reads+1&&await count("prepare_meeting_playback")===prepares&&await count("save_transcript_draft")===writes&&await count("start_diarization")===0,"read retry prepared or mutated analysis");
  results.push("failed track metadata is explicit with no guessed highlight; one read-only retry restores mapping without replay preparation, analysis or draft writes");
  await boot({imported:true});await play();await expectPlaying(["imported"]);
  check(await mode.inputValue()==="imported"&&await mode.locator("option").count()===1,"imported source fabricated recording modes");
  results.push("actual silent imported meeting playback highlights only its native imported track, never microphone-looking unknown IDs");

  await boot({palette:true});
  for(const width of [1280,960,360])for(const theme of ["light","dark"]){
    await page.setViewportSize({width,height:760});
    await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
    await detailUI.openSpeakerPanel();
    const evidence=await editor.locator(".speaker-label").evaluateAll((elements,theme)=>{
      const luminance=color=>{const rgb=color.match(/[\d.]+/g).slice(0,3).map(Number).map(n=>{const c=n/255;return c<=0.04045?c/12.92:((c+0.055)/1.055)**2.4;});return rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;};
      const colors=new Map(),tones=new Set();let minimum=Infinity;
      for(const e of elements){
        const style=getComputedStyle(e),fg=luminance(style.color),bg=luminance(style.backgroundColor),ratio=(Math.max(fg,bg)+0.05)/(Math.min(fg,bg)+0.05),id=e.dataset.speakerId,key=style.color+"/"+style.backgroundColor;
        if(colors.has(id)&&colors.get(id)!==key)throw Error("same speaker has different colors across surfaces");
        colors.set(id,key);tones.add(e.dataset.speakerTone);minimum=Math.min(minimum,ratio);
        const variable=style.getPropertyValue(`--speaker-${theme}-fg`).trim();
        if(!variable||style.color==="rgb(255, 255, 255)"&&theme==="light")throw Error("theme foreground missing");
      }
      return {minimum,tones:tones.size,speakers:colors.size,overflow:document.documentElement.scrollWidth>innerWidth};
    },theme);
    check(evidence.minimum>=4.5&&evidence.tones===12&&evidence.speakers===12&&!evidence.overflow,`palette contrast/consistency/width failed: ${JSON.stringify(evidence)}`);
    check(await editor.locator(".assigned-speakers .speaker-unset").count()===1,"unconfirmed speaker is conveyed only through color");
    await page.screenshot({path:`speaker-palette-${theme}-${width}.png`});
    await detailUI.closeSpeakerPanel();
    await page.screenshot({path:`speaker-transcript-${theme}-${width}.png`});
    results.push(`all 12 actual badge colors agree across names, assignments and transcript with contrast >=4.5 and name-unset text at ${width}px/${theme}`);
  }
  check(await count("start_capture")===0,"browser verification started recording");
  return {passed:results.length,results};
}
