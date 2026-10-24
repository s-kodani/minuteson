// Production mini no longer mounts help or inline attention UI.
async (page,capturePath=path=>path)=>{
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  await page.setViewportSize({width:227,height:101});
  await page.goto("http://127.0.0.1:1420/?window=recorder");
  await page.waitForFunction(()=>!!window.closeFixture);
  await page.evaluate(()=>window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID(),errorMessage:"合成入力エラー"}));
  await page.getByRole("button",{name:"停止",exact:true}).waitFor();
  const results=[];
  for(const theme of ["light","dark"]){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    if(await page.locator('.recorder-info-trigger,.recorder-operation-badge,.mini-source-label,.mini-waveform-note,.mini-journal-status,.mini-top strong').count())throw Error("mini retained text or help controls");
    if((await page.locator('.mini-shell').innerText()).match(/要対応|マイク|PC音声|録音中|未受信|確認不可/))throw Error("mini displayed an unnecessary status label");
    if(!await page.getByRole('button',{name:'停止',exact:true}).isEnabled())throw Error('stop disabled by removed attention UI');
    await page.screenshot({path:capturePath(`icon-only-${theme}.png`)});
    results.push(`${theme}: only timer, icons and waveform remain, even with an input error`);
  }
  return results;
}
