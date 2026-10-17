async(page,capturePath)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  await page.getByRole('region',{name:'発言本文',exact:true}).waitFor();
  await page.evaluate(()=>{
    // speaker-b has no original matching window. It models a corrected/manual
    // assignment and must nevertheless allow explicit enrollment.
    window.monVoiceFixture.candidates=Array.from({length:23},(_,i)=>({speakerId:'speaker-b',trackId:'microphone',startUs:i*30e6,endUs:i*30e6+(i<2?15e6:2e6),utteranceId:`manual-${i}`,text:`修正した話者の発言 ${i+1}`}));
  });
  await page.getByText('声の登録・照合',{exact:true}).click();
  const panel=page.locator('.voice-panel');
  const buttons=panel.getByRole('button',{name:'声を登録',exact:true});
  await page.waitForFunction(()=>document.querySelectorAll('.voice-speaker > button')[1]?.disabled===false);
  check(await buttons.first().isDisabled(),'speaker without candidates can register');
  check(await panel.locator('.voice-registration-reason').filter({hasText:'登録できる1秒以上の単独音声がありません。'}).count()===1,'missing candidate reason absent: '+JSON.stringify(await panel.locator('.voice-registration-reason').allTextContents()));
  await buttons.nth(1).click();
  const dialog=page.getByRole('dialog',{name:'声を人物として登録',exact:true});await dialog.waitFor();
  const boxes=dialog.getByRole('checkbox');
  check(await boxes.count()===20,'candidate page is not limited to 20');
  check(await boxes.nth(0).isChecked()&&await boxes.nth(1).isChecked(),'two longest candidates not initially selected');
  check(await boxes.nth(2).isDisabled(),'third candidate can be selected at maximum');
  await boxes.nth(0).uncheck();await boxes.nth(1).uncheck();
  check(await dialog.getByRole('button',{name:'声を登録',exact:true}).isDisabled(),'zero selections allow enrollment');
  await boxes.nth(2).focus();await page.keyboard.press('Space');
  check(await boxes.nth(2).isChecked(),'keyboard selection failed');
  await dialog.getByRole('button',{name:'次の20件',exact:true}).click();
  check(await boxes.count()===3,'last candidate page missing');
  await boxes.nth(0).check();
  await dialog.getByRole('button',{name:'候補21の登録区間を再生して確認',exact:true}).click();
  check(await page.evaluate(()=>{const r=window.monBrowserFixture.calls.filter(c=>c.command==='prepare_markdown_interval_playback').at(-1)?.args.request;return r.startUs===600e6&&r.endUs===602e6&&r.mode==='mic';}),'preview differs from selected interval');
  await dialog.getByRole('textbox',{name:'人物名',exact:true}).fill('シオツキ');
  await page.evaluate(()=>{window.monVoiceFixture.loseResponse=true;});
  await dialog.getByRole('button',{name:'声を登録',exact:true}).click();
  await dialog.getByRole('button',{name:'同じ登録を再送して確認',exact:true}).waitFor();
  check(await boxes.nth(0).isDisabled(),'uncertain submission permits changes');
  await dialog.getByRole('button',{name:'同じ登録を再送して確認',exact:true}).click();await dialog.waitFor({state:'hidden'});
  check(await page.evaluate(()=>{const r=window.monVoiceFixture.requests;return r.length===1&&r[0].speakerId==='speaker-b'&&r[0].selectedWindows.length===2&&r[0].selectedWindows[0].startUs===60e6&&r[0].selectedWindows[1].startUs===600e6;}),'submitted selection changed or duplicate job created');
  results.push('manual assignment candidates support pagination, keyboard selection, exact preview and immutable lost-reply retry');
  await page.evaluate(()=>window.monVoiceFixture.complete());
  await page.waitForFunction(()=>document.querySelectorAll('.voice-speaker > button')[1]?.disabled===false);
  await buttons.nth(1).click();await dialog.waitFor();
  // A definitive rejection reloads changed candidates and requires review.
  await page.evaluate(()=>{window.monVoiceFixture.candidates=window.monVoiceFixture.candidates.slice(1);window.monVoiceFixture.contextRevision++;window.monVoiceFixture.failStart=true;});
  await dialog.getByRole('button',{name:'声を登録',exact:true}).click();
  await dialog.getByText('登録候補が変更されました。区間を選び直すか、残った選択を確認してください。',{exact:true}).waitFor();
  check(await dialog.getByRole('button',{name:'声を登録',exact:true}).isDisabled(),'stale selection permits enrollment');
  await dialog.getByRole('button',{name:'選択区間を確認しました',exact:true}).click();
  await page.evaluate(()=>{window.monVoiceFixture.failStart=false;});
  check(await dialog.getByRole('button',{name:'声を登録',exact:true}).isEnabled(),'review did not restore valid selection');
  results.push('removed candidate blocks enrollment until the remaining selection is explicitly reviewed');
  await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
  await page.evaluate(()=>window.monVoiceFixture.candidates.push({speakerId:'speaker-a',trackId:'microphone',startUs:0,endUs:15e6,utteranceId:'saved-correction',text:'保存後の候補'}));
  await page.getByRole('button',{name:'田中の名前を変更',exact:true}).click();
  const rename=page.getByRole('dialog',{name:'話者名を変更',exact:true});
  await rename.getByRole('textbox',{name:'表示名',exact:true}).fill('確認用の田中');
  await rename.getByRole('button',{name:'保存',exact:true}).click();await rename.waitFor({state:'hidden'});
  await page.waitForFunction(()=>document.querySelectorAll('.voice-speaker > button')[0]?.disabled===false);
  results.push('saving transcript edits refreshes candidates in the expanded panel without closing it');
  for(const width of [1280,390]){
    await page.setViewportSize({width,height:900});
    if(width<760){await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));const drawer=page.getByRole('dialog',{name:'話者一覧',exact:true});if(!await drawer.isVisible())await page.getByRole('button',{name:'話者一覧を開く',exact:true}).click();await drawer.waitFor();}
    await buttons.nth(1).click();await dialog.waitFor();
    await dialog.evaluate(el=>el.scrollTop=0);
    check(await dialog.evaluate(el=>el.clientWidth>0&&el.scrollWidth<=el.clientWidth),'candidate dialog hidden or overflowing');
    await page.screenshot({path:capturePath(`voice-selection-${width}.png`)});
    await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
    if(width<760)await page.getByRole('button',{name:'話者一覧を閉じる',exact:true}).click();
  }
  results.push('candidate selection fits desktop and narrow screens');
  return {passed:results.length,results};
}
