// Synthetic timeline only: never starts microphone capture or audio playback.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const editor = page.getByRole("region", {name:"文字起こし編集",exact:true});
  await page.getByRole("button", {name:"会議A",exact:true}).click();
  await page.getByRole("button", {name:"再生設定",exact:true}).click();
  const modal = page.getByRole("dialog", {name:"再生設定",exact:true});
  await modal.waitFor();
  check(await modal.getByRole("combobox").count() === 1, "playback settings must contain only track selection");
  check(await modal.getByRole("checkbox").count() === 0, "removed following control remains");
  await modal.getByRole("button", {name:"確認を閉じる",exact:true}).click();
  results.push("playback settings have no removed display controls");
  await editor.getByRole("button", {name:"00:00:00の発言を編集",exact:true}).click();
  const input = editor.getByRole("textbox", {name:"00:00:00の発言",exact:true});
  await input.fill("未保存の点検日を保持する");
  const before = await input.evaluate(el => {
    el.setSelectionRange(2,5);
    window.monHighlightInput = el;
    return {start:el.selectionStart,end:el.selectionEnd,scroll:el.closest(".utterance-list").scrollTop};
  });
  await page.evaluate(async () => {
    const {playbackTimeline} = await import("/src/playback-follow.ts");
    playbackTimeline.publish({owner:"synthetic-check",session:"check",meetingId:"meeting-0",analysisId:"analysis",utteranceId:"speech-a",scope:"utterance",timeUs:1_000_000,playing:true,trackKind:"mic"});
  });
  await editor.locator('[data-utterance-id="speech-a"].utterance-playing').waitFor();
  for (const colorScheme of ["light","dark"]) await page.emulateMedia({colorScheme});
  check(await input.evaluate((el, expected) => el === window.monHighlightInput && document.activeElement === el
    && el.value === "未保存の点検日を保持する" && el.selectionStart === expected.start && el.selectionEnd === expected.end
    && el.closest(".utterance-list").scrollTop === expected.scroll, before), "highlight/theme moved the editor, selection or scroll");
  results.push("synthetic playback emphasis and theme preserve input focus selection and scroll");
  await page.evaluate(async () => { (await import("/src/playback-follow.ts")).playbackTimeline.release("synthetic-check"); });
  await page.waitForFunction(() => !document.querySelector(".utterance-playing"));
  check(await input.inputValue() === "未保存の点検日を保持する", "stopping emphasis lost pending text");
  await editor.getByRole("button", {name:"キャンセル",exact:true}).click();
  results.push("ending emphasis preserves pending text and individual editing remains available");
  const nav = page.getByRole("navigation", {name:"メインナビゲーション",exact:true});
  await nav.getByRole("button", {name:"会議一覧",exact:true}).click();
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({length:103}, (_,i) => ({
    id:`synthetic-${i}`,trackId:"microphone",startUs:i*1_000_000,endUs:(i+1)*1_000_000,
    recognizedText:`発言${i}`,effectiveText:`発言${i}`,attributionState:"assigned",speakerIds:[i%2?"speaker-b":"speaker-a"]
  }))));
  await page.getByRole("button", {name:"会議A",exact:true}).click();
  check(await editor.locator('[data-utterance-id]').count()===103&&await editor.locator('.transcript-pages').count()===0,"utterances remain paginated");
  const jump=editor.getByRole('button',{name:'再生位置へ移動',exact:true});
  check(await jump.isDisabled(),'jump enabled without a timeline');
  const heading=await editor.getByRole('heading',{name:'発言一覧',exact:true}).boundingBox(),button=await jump.boundingBox();
  check(button.x>=heading.x+heading.width,'jump is not to the right of the heading');
  await editor.getByRole('button',{name:'00:00:00の発言を編集',exact:true}).click();
  const pending=editor.getByRole('textbox',{name:'00:00:00の発言',exact:true});await pending.fill('ジャンプ中も保持する本文');
  await editor.getByRole('searchbox',{name:'発言を検索',exact:true}).fill('発言0');
  const saved=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='save_transcript_draft').length);
  await page.evaluate(async()=>{(await import('/src/playback-follow.ts')).playbackTimeline.publish({owner:'synthetic-check',session:'jump',meetingId:'meeting-0',analysisId:'analysis',utteranceId:'synthetic-102',scope:'utterance',timeUs:102_500_000,playing:true,trackKind:'mic'});});
  await page.waitForFunction(()=>!document.querySelector('.transcript-list-heading button')?.disabled);
  check(await editor.locator('[data-utterance-id="synthetic-102"]').count()===0,'timeline automatically cleared the filter');
  await jump.click();await editor.locator('[data-utterance-id="synthetic-102"]').waitFor();
  check(await editor.getByRole('searchbox',{name:'発言を検索',exact:true}).inputValue()===''&&await editor.locator('[data-utterance-id]').count()===103,'explicit jump did not reveal all matching rows');
  const target=editor.locator('[data-utterance-id="synthetic-102"]');
  check(await target.evaluate(el=>{const r=el.getBoundingClientRect();return !el.classList.contains('utterance-group-fragment')&&r.height>0&&r.top>=0&&r.bottom<=innerHeight;}),'jump did not scroll to the target');
  check(await pending.inputValue()==='ジャンプ中も保持する本文'&&await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='save_transcript_draft').length)===saved,'jump saved or lost pending input');
  results.push('heading-right jump reveals a filtered playback target without saving or losing pending text');
  await page.evaluate(async()=>{(await import('/src/playback-follow.ts')).playbackTimeline.publish({owner:'synthetic-check',session:'jump',meetingId:'meeting-0',analysisId:'analysis',utteranceId:'synthetic-50',scope:'utterance',timeUs:50_500_000,playing:false,trackKind:'mic'});});
  await jump.click();check(await editor.locator('[data-utterance-id="synthetic-50"]').evaluate(el=>{const r=el.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight;}),'paused timeline cannot be jumped to');
  await page.evaluate(async()=>{(await import('/src/playback-follow.ts')).playbackTimeline.publish({owner:'synthetic-check',session:'foreign',meetingId:'another-meeting',analysisId:'analysis',utteranceId:'synthetic-50',scope:'utterance',timeUs:50_500_000,playing:true,trackKind:'mic'});});
  check(await jump.isDisabled(),'another meeting enabled jump');
  await page.evaluate(async()=>{(await import('/src/playback-follow.ts')).playbackTimeline.release('synthetic-check');});
  check(await jump.isDisabled(),'released timeline left jump enabled');
  results.push('jump supports paused positions and rejects other meetings or released playback');
  return results;
}
