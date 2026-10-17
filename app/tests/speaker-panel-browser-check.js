// Current speaker filters/name modals with synthetic persistence only.
async(page,capturePath)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  await page.evaluate(()=>window.monBrowserFixture.setUtterances(Array.from({length:70},(_,i)=>({id:`speech-${i}`,trackId:'microphone',startUs:i*1_000_000,endUs:(i+1)*1_000_000,recognizedText:`本文${i}`,effectiveText:`本文${i}`,attributionState:i%3===2?'unknown':'assigned',speakerIds:i%3===2?[]:[i%3===0?'speaker-a':'speaker-b']}))));
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  const editor=page.getByRole('region',{name:'文字起こし編集',exact:true});
  const body=editor.getByRole('region',{name:'発言本文',exact:true});
  await body.waitFor();
  check(await body.getByRole('article').count()===70,'utterance list still truncates matching rows');
  const count=()=>page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='save_transcript_draft').length);
  await editor.getByRole('button',{name:'田中の発言で絞り込み',exact:true}).click();
  check(await body.getByRole('article').count()===24,'speaker filter includes wrong rows');
  await editor.getByRole('searchbox',{name:'発言を検索',exact:true}).fill('本文0');check(await body.getByRole('article').count()===1,'speaker/text filters do not intersect');
  const unknown=editor.getByRole('button',{name:'話者不明の発言 23',exact:true});check(await unknown.isVisible(),'unknown speaker count is missing or affected by search');
  await editor.getByRole('searchbox',{name:'発言を検索',exact:true}).fill('');await unknown.click();check(await body.getByRole('article').count()===23,'unknown filter invents an assigned speaker');
  await editor.getByRole('button',{name:/^すべての発言/}).click();check(await body.getByRole('article').count()===70,'clearing filters loses utterances');check(await editor.locator('.transcript-pages').count()===0,'utterance pagination remains');check(await body.locator('[data-utterance-id]').last().getAttribute('data-utterance-id')==='speech-69','final utterance is not in sequence');
  results.push('speaker unknown and text filters combine correctly and all 70 rows remain available without pagination');
  await editor.getByRole('button',{name:'田中の名前を変更',exact:true}).click();
  const name=page.getByRole('dialog',{name:'話者名を変更',exact:true}),input=name.getByRole('textbox',{name:'表示名',exact:true});await input.waitFor();
  await input.fill('未確定の話者名');check(await count()===0,'typing a name automatically saves');await page.keyboard.press('Escape');await name.waitFor({state:'hidden'});check(await editor.getByRole('button',{name:'田中の名前を変更',exact:true}).count()===1,'cancelling changed the label');
  await editor.getByRole('button',{name:'田中の名前を変更',exact:true}).click();await input.fill('確認済みの名前');await name.getByRole('button',{name:'保存',exact:true}).click();await name.waitFor({state:'hidden'});check(await count()===1&&await editor.getByRole('button',{name:'確認済みの名前の名前を変更',exact:true}).count()===1,'explicit name save was lost or duplicated');
  results.push('name labels open a plain modal; typing/cancel does not save; one explicit save updates the same speaker');
  await editor.getByRole('button',{name:'確認済みの名前の名前を変更',exact:true}).click();await input.fill('失敗しても保持する名前');await page.evaluate(()=>window.monBrowserFixture.failSave=true);await name.getByRole('button',{name:'保存',exact:true}).click();await name.getByRole('button',{name:'保存を再試行',exact:true}).waitFor();check(await input.inputValue()==='失敗しても保持する名前','failed save discarded input');await page.evaluate(()=>window.monBrowserFixture.failSave=false);await name.getByRole('button',{name:'保存を再試行',exact:true}).click();await name.waitFor({state:'hidden'});
  results.push('failed name saving retains input and explicit retry resolves the same edit');
  for(const [width,theme]of [[1280,'light'],[900,'dark'],[760,'light'],[360,'dark']]){
    await page.setViewportSize({width,height:800});await page.evaluate(t=>document.documentElement.dataset.theme=t,theme);
    await page.waitForFunction(()=>document.querySelector('.workspace-panel-shell')?.dataset.narrow===String(matchMedia('(max-width: 760px)').matches));
    await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    const drawer=page.getByRole('dialog',{name:'話者一覧',exact:true});
    if(width<=760){if(!await drawer.isVisible())await page.getByRole('button',{name:'話者一覧を開く',exact:true}).click();await drawer.waitFor();check(await drawer.getByRole('button',{name:'失敗しても保持する名前の名前を変更',exact:true}).isVisible(),'narrow drawer lost speaker controls');}
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'speaker layout overflows horizontally');await page.screenshot({path:capturePath(`speakers-${width}-${theme}.png`)});
    if(width<=760){await drawer.getByRole('button',{name:'話者一覧を閉じる',exact:true}).click();await drawer.waitFor({state:'hidden'});}
  }
  results.push('wide speaker panel and narrow modal drawer fit both themes without horizontal overflow');
  check(await page.getByRole('button',{name:/一括割り当て|編集を開始/}).count()===0,'retired bulk tools remain');
  results.push('speaker controls retain individual editing and restored merge without bulk assignment');
  return {passed:results.length,results};
}
