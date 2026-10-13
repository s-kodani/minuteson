// Synthetic timeline only: never starts microphone capture or audio playback.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const editor = page.getByRole("region", {name:"文字起こし編集",exact:true});
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
    recognizedText:`発言${i}`,effectiveText:`発言${i}`,attributionState:"assigned",speakerIds:["speaker-a"]
  }))));
  await page.getByRole("button", {name:"会議A",exact:true}).click();
  await editor.getByRole("button", {name:"次へ",exact:true}).click();
  await editor.locator('[data-utterance-id="synthetic-50"]').waitFor();
  await page.evaluate(async () => {
    (await import("/src/playback-follow.ts")).playbackTimeline.publish({owner:"synthetic-check",session:"check",meetingId:"meeting-0",analysisId:"analysis",utteranceId:"synthetic-0",scope:"utterance",timeUs:500_000,playing:true,trackKind:"mic"});
  });
  check(await editor.locator('[data-utterance-id="synthetic-50"]').count() === 1
    && await editor.locator('[data-utterance-id="synthetic-0"]').count() === 0, "playback changed the manually selected page");
  await editor.getByRole("button", {name:"前へ",exact:true}).click();
  await editor.locator('[data-utterance-id="synthetic-0"].utterance-playing').waitFor();
  await page.evaluate(async () => { (await import("/src/playback-follow.ts")).playbackTimeline.release("synthetic-check"); });
  results.push("playback never changes pages and manual pagination remains available");
  return results;
}
