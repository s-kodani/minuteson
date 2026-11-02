// Production UI with synthetic voices and silent PCM; no user DB or sound.
async (page,capturePath) => {
  const results=[],check=(value,message)=>{if(!value)throw Error(message);};
  const nav=page.getByRole('navigation',{name:'メインナビゲーション',exact:true});
  const library=page.getByRole('region',{name:'ボイス一覧',exact:true});
  const calls=command=>page.evaluate(command=>window.monBrowserFixture.calls.filter(c=>c.command===command),command);
  await nav.getByRole('button',{name:'ボイス一覧',exact:true}).click();
  await library.getByText('登録した話者はいません。',{exact:true}).waitFor();
  check(await nav.getByRole('button',{name:'ボイス一覧',exact:true}).getAttribute('aria-current')==='page','voice navigation is not selected');
  check((await calls('prepare_voice_sample_playback')).length===0,'opening the library started playback');
  results.push('sidebar opens the voice library with an empty state and no automatic playback');

  await page.evaluate(()=>{
    const sample=(id,startUs,endUs,patch={})=>({id,modelHash:'a'.repeat(64),extractorVersion:'test',sourceMeetingId:'meeting-0',sourceAvailable:true,audioAvailable:true,audioStorageState:"ready",audioStorageError:null,startUs,endUs,createdAt:0,...patch});
    window.monVoiceFixture.people=[
      {id:'person-a',name:'田中',version:1,samples:[sample('voice-a',1_500_000,4_500_000,{sourceAvailable:false,sourceMeetingId:null}),sample('voice-missing',0,2_000_000,{sourceAvailable:false,audioAvailable:false,audioStorageState:"unavailable",sourceMeetingId:null})]},
      {id:'person-b',name:'佐藤',version:1,samples:[sample('voice-imported',7_000_000,11_000_000,{sourceMeetingId:'meeting-1',trackKind:'imported'})]}
    ];
    window.monBrowserFixture.preparation.strict=true;
    window.monBrowserFixture.preparation.responsive=true;
  });
  await library.getByRole('button',{name:'ボイス一覧を再読込',exact:true}).click();
  const tanaka=library.getByRole('region',{name:'田中',exact:true}),sato=library.getByRole('region',{name:'佐藤',exact:true});
  await tanaka.waitFor();await sato.waitFor();
  check(await tanaka.getByRole('button',{name:'田中の登録区間2を再生',exact:true}).isDisabled(),'missing source can be played');
  await tanaka.getByText('登録元の音声がないため再生できません。',{exact:true}).waitFor();
  await tanaka.getByRole('button',{name:'田中の登録区間1を再生',exact:true}).click();
  const player=tanaka.getByRole('group',{name:'登録した声の再生',exact:true});
  await player.getByRole('button',{name:'一時停止',exact:true}).waitFor();
  check(await page.evaluate(()=>{
    const f=window.monBrowserFixture,c=f.calls.find(c=>c.command==='prepare_voice_sample_playback'),info=Object.values(f.meetingAudio.sessions)[0];
    return c.args.sampleId==='voice-a'&&Object.keys(c.args).sort().join(',')==='preparationId,sampleId'&&info.startUs===1_500_000&&info.endUs===4_500_000&&info.trackKind==='processed';
  }),'playback did not use only the saved sample identity and interval');
  await player.getByRole('button',{name:'一時停止',exact:true}).click();
  await player.getByRole('button',{name:'再生',exact:true}).waitFor();
  await player.getByRole('slider',{name:'登録区間内の再生位置',exact:true}).focus();
  await page.keyboard.press('Home');await page.keyboard.press('ArrowRight');
  await player.getByRole('combobox',{name:'速度',exact:true}).selectOption('1.5');
  results.push('registered people list their samples; available samples play the saved interval with pause seek and speed controls');

  await sato.getByRole('button',{name:'佐藤の登録区間1を再生',exact:true}).click();
  await sato.getByRole('group',{name:'登録した声の再生',exact:true}).getByRole('button',{name:'一時停止',exact:true}).waitFor();
  check(await library.getByRole('group',{name:'登録した声の再生',exact:true}).count()===1,'multiple sample players remain visible');
  await page.waitForFunction(()=>Object.values(window.monBrowserFixture.meetingAudio.sessions).length===1);
  check(await page.evaluate(()=>Object.values(window.monBrowserFixture.meetingAudio.sessions)[0].trackKind==='processed'),'another source track was mixed into the imported sample');
  await nav.getByRole('button',{name:'会議一覧',exact:true}).click();
  await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length===0);
  await page.getByRole('button',{name:'前の画面へ戻る',exact:true}).click();
  await tanaka.waitFor();
  results.push('switching samples closes the previous token; leaving stops playback and history returns to the library');

  await page.evaluate(()=>{window.monVoiceFixture.failPeople=true;});
  await library.getByRole('button',{name:'ボイス一覧を再読込',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'合成のボイス一覧取得失敗'}).waitFor();
  check(await tanaka.isVisible(),'read failure cleared the previous list');
  await page.evaluate(()=>{window.monVoiceFixture.failPeople=false;window.monVoiceFixture.corruptSample={endUs:999999};});
  await library.getByRole('button',{name:'ボイス一覧を再読込',exact:true}).click();
  const reads=(await calls('read_playback_chunk')).length;
  await tanaka.getByRole('button',{name:'田中の登録区間1を再生',exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.notifications().some(n=>n.message.includes('確認した登録区間')));
  check((await calls('read_playback_chunk')).length===reads,'invalid sample response read PCM');
  await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length===0);
  await page.evaluate(()=>{window.monVoiceFixture.corruptSample=null;window.monVoiceFixture.holdSample=true;});
  await tanaka.getByRole('button',{name:'田中の登録区間1を再生',exact:true}).click();
  await page.waitForFunction(()=>window.monVoiceFixture.pendingSamples.length===1);
  await player.getByRole('button',{name:'再生準備を取り消す',exact:true}).click();
  await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.preparation.reservations).length===0);
  check((await calls('read_playback_chunk')).length===reads,'cancelled preparation read PCM');
  await page.evaluate(()=>{window.monVoiceFixture.holdSample=false;window.monVoiceFixture.pendingSamples=[];});
  results.push('failed list refresh retains rows; mismatched or cancelled audio preparations release without reading PCM');

  await tanaka.getByRole('button',{name:'名前を変更',exact:true}).click();
  const rename=page.getByRole('dialog',{name:'登録した人物名を変更',exact:true});
  await rename.getByRole('textbox',{name:'人物名',exact:true}).fill('登録者の田中');
  await rename.getByRole('button',{name:'保存',exact:true}).click();await rename.waitFor({state:'hidden'});
  const renamed=library.getByRole('region',{name:'登録者の田中',exact:true});await renamed.waitFor();
  await renamed.getByRole('button',{name:'登録者の田中の登録区間1を再生',exact:true}).click();
  await renamed.getByRole('group',{name:'登録した声の再生',exact:true}).getByRole('button',{name:'一時停止',exact:true}).waitFor();
  await renamed.getByRole('button',{name:'この声を削除',exact:true}).first().click();
  await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length===0);
  check(await renamed.getByRole('group',{name:'登録した声の再生',exact:true}).count()===0,'deleting a sample kept its player');
  check(await renamed.getByRole('button',{name:'登録者の田中の登録区間1を再生',exact:true}).isDisabled(),'deleted sample was replaced with another playable range');
  results.push('person renaming persists and deleting the playing sample stops audio before removing it');

  for(const width of [1280,390]){
    await page.setViewportSize({width,height:900});
    await sato.getByRole('button',{name:'佐藤の登録区間1を再生',exact:true}).click();
    await sato.getByRole('group',{name:'登録した声の再生',exact:true}).getByRole('button',{name:'一時停止',exact:true}).waitFor();
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'voice library overflows the screen');
    check(await sato.evaluate(el=>el.scrollWidth<=el.clientWidth),'sample controls overflow the person card');
    await page.screenshot({path:capturePath(`voice-library-${width}.png`)});
  }
  await nav.getByRole('button',{name:'会議一覧',exact:true}).click();
  await page.waitForFunction(()=>Object.keys(window.monBrowserFixture.meetingAudio.sessions).length===0);
  check((await calls('start_voice_job')).length===0,'library actions enrolled or re-extracted voices');
  results.push('the library and player fit desktop and narrow screens without retraining voices');
  await page.setViewportSize({width:1280,height:900});
  await page.getByRole('button',{name:'会議A',exact:true}).click();
  const editor=page.getByRole('region',{name:'文字起こし編集',exact:true});
  await editor.getByRole('button',{name:'00:00:00の発言を編集',exact:true}).click();
  const pending=editor.getByRole('textbox',{name:'00:00:00の発言',exact:true});
  await pending.fill('ボイス一覧への移動前の未保存本文');
  await page.evaluate(()=>{window.monBrowserFixture.failSave=true;});
  await nav.getByRole('button',{name:'ボイス一覧',exact:true}).click();
  await editor.getByRole('button',{name:'保存を再試行',exact:true}).waitFor();
  check(await library.count()===0&&await pending.inputValue()==='ボイス一覧への移動前の未保存本文','voice navigation discarded a failed draft save');
  await page.evaluate(()=>{window.monBrowserFixture.failSave=false;});
  await editor.getByRole('button',{name:'保存を再試行',exact:true}).click();
  await pending.waitFor({state:'hidden'});
  await nav.getByRole('button',{name:'ボイス一覧',exact:true}).click();
  await renamed.waitFor();
  results.push('voice navigation waits for draft saving and preserves failed edits for retry');
  await page.evaluate(()=>{
    const s=window.monVoiceFixture.people[0].samples[0];
    Object.assign(s,{audioStorageState:"failed",audioAvailable:false,audioStorageError:"合成の音声保存失敗"});
    window.monVoiceFixture.failAudioStorage=true;
  });
  await library.getByRole('button',{name:'ボイス一覧を再読込',exact:true}).click();
  await renamed.getByText('登録音声の保存に失敗しました。合成の音声保存失敗',{exact:true}).waitFor();
  await library.getByRole('button',{name:'音声保存・削除を再試行',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'合成の音声保存失敗'}).waitFor();
  await page.evaluate(()=>{window.monVoiceFixture.failAudioStorage=false;});
  await library.getByRole('button',{name:'音声保存・削除を再試行',exact:true}).click();
  await renamed.getByText('登録音声は保存済みです。元の会議を削除しても再生できます。',{exact:true}).waitFor();
  check(await renamed.getByRole('button',{name:'登録者の田中の登録区間1を再生',exact:true}).isEnabled(),'saved audio remains disabled after retry');
  results.push('saved clips play without their source meeting and failed preservation can be retried from the library');
  return {passed:results.length,results};
}
