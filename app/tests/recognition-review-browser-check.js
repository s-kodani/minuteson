async (page, capturePath = path => path) => {
  const range={trackId:"microphone",startUs:3000000,endUs:7000000,reason:"repetition_after_recovery"};
  await page.evaluate(range=>window.monBrowserFixture.setRecognitionReview([range]),range);
  const row=page.locator(".meeting-table tbody tr").filter({hasText:"会議A"}).first();
  await row.locator(".meeting-row-actions > button").click();
  await row.getByRole("menuitem",{name:"開く",exact:true}).click();
  const review=page.getByRole("region",{name:"文字起こしの要確認区間"});
  await review.getByText(/完了・要確認あり/).waitFor();
  for (const width of [1360, 400]) {
    await page.setViewportSize({ width, height: 900 });
    if (!await review.evaluate(section => {
      const status = section.querySelector("p").getBoundingClientRect();
      const ranges = section.querySelector("ul").getBoundingClientRect();
      const search = section.nextElementSibling.querySelector("input").getBoundingClientRect();
      return Math.abs(ranges.top - status.bottom - 8) <= 1 && Math.abs(search.top - ranges.bottom - 12) <= 1
        && Math.abs(status.left - search.left) <= 1;
    })) throw Error(`review ranges and search spacing differ at ${width}px`);
    await page.screenshot({ path: capturePath(`recognition-review-${width}.png`) });
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  await review.getByRole("button",{name:"00:00:03–00:00:07"}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==="prepare_meeting_playback"&&c.args.request.startReference?.startUs===3000000));
  await page.evaluate(range=>window.monBrowserFixture.setRecognitionReview([range],true),range);
  await review.getByText(/認識できた本文はありません/).waitFor();
  if(await page.locator(".transcript-body-edit").count())throw Error("review marker became recognized speech");
  return {passed:2,results:["completed recognition review ranges play the exact original interval","all-unrecognized results show an explicit empty transcript without inventing utterances"]};
}
