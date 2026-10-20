// Current highlight consumer with synthetic timeline events, not acoustic QA.
async(page,capturePath,detailUI)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const boot=async(failed=false)=>{
    await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
    await page.evaluate(failed=>{const f=window.monBrowserFixture;f.failTracks=failed;f.trackPlan={tracks:[{id:'native-mic',kind:'mic',startUs:0,endUs:15_000_000},{id:'native-system',kind:'system',startUs:0,endUs:15_000_000}]};f.setUtterances(['mic','system','unknown'].map((id,i)=>({id,trackId:['native-mic','native-system','missing'][i],startUs:0,endUs:15_000_000,recognizedText:id,effectiveText:id,attributionState:'assigned',speakerIds:[i===1?'speaker-b':'speaker-a']})));},failed);
    await page.getByRole('button',{name:'会議A',exact:true}).click();
    await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='get_meeting_analysis_audio'));
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  };
  const publish=extra=>page.evaluate(async extra=>window.monBrowserFixture.publishPlaybackTimeline({owner:'synthetic-colour',session:'colour',meetingId:'meeting-0',analysisId:null,utteranceId:null,scope:'meeting',timeUs:1_000_000,playing:true,trackKind:'mic',...extra}),extra);
  const emphasized=()=>page.locator('.utterance-playing').evaluateAll(rows=>rows.map(r=>r.dataset.utteranceId).sort());
  await boot();await publish({trackKind:'mic'});await page.locator('[data-utterance-id="mic"].utterance-playing').waitFor();check(JSON.stringify(await emphasized())==='["mic"]','mic highlighted another track');results.push('mic timeline highlights only utterances with verified mic track identity');
  await publish({trackKind:'system'});await page.locator('[data-utterance-id="system"].utterance-playing').waitFor();check(JSON.stringify(await emphasized())==='["system"]','PC highlighted mic or unknown track');results.push('PC timeline highlights only verified system-track rows');
  await publish({trackKind:'both'});await page.waitForFunction(()=>document.querySelectorAll('.utterance-playing').length===2);check(JSON.stringify(await emphasized())==='["mic","system"]','mixed playback inferred an unknown track');results.push('mixed playback highlights both verified tracks but never guesses missing track metadata');
  for(const extra of [{playing:false},{meetingId:'another-meeting'},{trackKind:'invalid'},{timeUs:15_000_000}]){await publish(extra);await page.waitForFunction(()=>!document.querySelector('.utterance-playing'));}
  results.push('pause foreign meeting invalid kind and exact end remove emphasis instead of retaining a stale playing claim');
  await publish({trackKind:'both'});await page.waitForFunction(()=>document.querySelectorAll('.utterance-playing').length===2);
  const label=page.locator('[data-utterance-id="mic"] .speaker-label').first(),tone=await label.getAttribute('data-speaker-tone');
  for(const theme of ['light','dark']){await detailUI.applyPlaybackTheme(theme);check(await label.getAttribute('data-speaker-tone')===tone&&JSON.stringify(await emphasized())==='["mic","system"]','theme changed identity or emphasis');await page.screenshot({path:capturePath(`playing-speakers-${theme}.png`)});}
  check(await page.evaluate(()=>window.monBrowserFixture.calls.every(c=>c.command!=='save_transcript_draft')),'highlighting saved user content');results.push('themes retain stable speaker ID tones and emphasis without saving content');
  await page.evaluate(async()=>window.monBrowserFixture.releasePlaybackTimeline('synthetic-colour'));
  await boot(true);await publish({trackKind:'both'});check(await emphasized().then(v=>v.length)===0,'failed metadata lookup guessed tracks');results.push('failed track lookup never produces inferred speaker playback colours');
  await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
  await page.evaluate(()=>{const f=window.monBrowserFixture;f.trackPlan={tracks:[{id:'meeting-mix',kind:'processed',startUs:0,endUs:15_000_000}]};f.setUtterances(['meeting','unknown'].map(id=>({id,trackId:id==='meeting'?'meeting-mix':'missing',startUs:0,endUs:15_000_000,recognizedText:id,effectiveText:id,attributionState:'assigned',speakerIds:['speaker-a']})));});
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='get_meeting_analysis_audio'));
  for(const kind of ['mic','system','both','processed']){
    await publish({trackKind:kind});await page.locator('[data-utterance-id="meeting"].utterance-playing').waitFor();
    check(JSON.stringify(await emphasized())==='["meeting"]','common meeting time did not follow the selected playback source');
  }
  results.push('shared meeting rows follow microphone PC mixed and common-WAV playback using the same meeting time');
  return {passed:results.length,results};
}
