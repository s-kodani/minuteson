// Real current controls; fixture IPC serves silent PCM, never native capture.
async(page,capturePath,detailUI)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const nav=page.locator('.sidebar'),dock=page.getByRole('region',{name:'会議の共通音声プレーヤー',exact:true});
  const editor=page.getByRole('region',{name:'文字起こし編集',exact:true});
  const row=editor.locator('[data-utterance-id="speech-a"]');
  const player=()=>dock.getByRole('group',{name:'会議全体の音声再生',exact:true});
  const open=async()=>{await nav.getByRole('button',{name:'会議一覧',exact:true}).click();await page.locator('.meeting-name').first().click();await row.waitFor();};
  const boot=async()=>{await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);await open();};
  const calls=c=>page.evaluate(c=>window.monBrowserFixture.calls.filter(x=>x.command===c).length,c);
  const play=async()=>{await row.getByRole('button',{name:'音声を再生',exact:true}).click();await player().getByRole('button',{name:'一時停止',exact:true}).waitFor();};
  const pause=async()=>{await player().getByRole('button',{name:'一時停止',exact:true}).click();await player().getByRole('button',{name:'再生',exact:true}).waitFor();};
  await page.reload();await page.waitForFunction(()=>!!window.monBrowserFixture);
  check(!await dock.isVisible(),'unselected meeting exposes a player');await open();
  check(await dock.isVisible()&&await calls('prepare_meeting_playback')===0&&await player().getByRole('button',{name:'再生',exact:true}).isVisible(),'idle selection prepares or plays audio');
  results.push('selecting a meeting shows its dock without automatically preparing or playing audio');
  await play();check((await row.getAttribute('class')).includes('utterance-playing'),'playing row is not emphasized');await pause();
  check(await player().evaluate(e=>!e.closest('.content')&&!e.closest('.transcript-card'))&&!(await dock.innerText()).includes('同期未検証'),'player scrolls with body or includes an extra status row');
  results.push('utterance playback uses the shared dock and keeps alignment information out of its height');
  check(await page.evaluate(async()=>{const {toasts}=await import('/src/toast.tsx');return [...toasts.getSnapshot(),...toasts.getHistorySnapshot()].every(t=>!t.key.includes('playback-alignment')&&!t.message.includes('同期未検証'));}),'utterance playback published an unwanted synchronization toast or history entry');results.push('utterance playback emits no synchronization toast or notification history entry');
  const fullSlider=player().getByRole('slider',{name:'会議全体の再生位置',exact:true});
  check(await fullSlider.getAttribute('max')==='57600000','utterance clipped the full timeline');
  await fullSlider.fill('238400');await player().getByRole('button',{name:'再生',exact:true}).click();
  await editor.locator('[data-utterance-id="speech-b"].utterance-playing').waitFor();await pause();
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==='prepare_meeting_playback').at(-1).args.request.startReference?.utteranceId)==='speech-a','following the next utterance changed the selected reference');
  results.push('playback crosses the selected utterance end and highlights the next utterance on the whole timeline');
  await player().evaluate(e=>window.monBrowserFixture.playerNode=e);const prepared=await calls('prepare_meeting_playback');
  for(const width of [1280,1200,960,360]){
    await page.setViewportSize({width,height:800});
    await editor.locator('.utterance-list').evaluate(e=>e.scrollTop=e.scrollHeight);
    check(await player().evaluate(e=>e===window.monBrowserFixture.playerNode),'resize or scrolling remounted player');
    check(await dock.evaluate(e=>{const r=e.getBoundingClientRect();return r.bottom<=innerHeight+1&&e.scrollWidth<=e.clientWidth+1&&document.documentElement.scrollWidth<=innerWidth;}),'dock overlaps or horizontally overflows');
  }
  check(await calls('prepare_meeting_playback')===prepared,'resize prepared another token');results.push('desktop and narrow layouts retain the same visible full-meeting player and token');
  const slider=()=>player().getByRole('slider',{name:'会議全体の再生位置',exact:true});
  await slider().fill('144000');await player().getByRole('combobox',{name:'速度',exact:true}).selectOption('1.5');
  for(const theme of ['light','dark']){await detailUI.applyPlaybackTheme(theme);check(await slider().inputValue()==='144000'&&await player().getByRole('combobox',{name:'速度',exact:true}).inputValue()==='1.5','theme reset playback state');await page.screenshot({path:capturePath(`fixed-${theme}.png`)});}
  results.push('theme changes retain paused position speed and native token');
  for(const palette of ['purple','monochrome']){
    await page.evaluate(async colorTheme=>{const f=window.monBrowserFixture;f.uiPreferences.payload.colorTheme=colorTheme;f.uiPreferences.lockVersion++;await f.emitUiPreferences();},palette);
    await page.waitForFunction(palette=>document.documentElement.dataset.colorTheme===palette,palette);
    check(await slider().inputValue()==='144000'&&await player().getByRole('combobox',{name:'速度',exact:true}).inputValue()==='1.5'&&await player().evaluate(e=>e===window.monBrowserFixture.playerNode)&&await calls('prepare_meeting_playback')===prepared,'color change reset position speed player or token');
  }
  results.push('color changes retain paused position speed player node and native token');
  await row.getByRole('button',{name:'00:00:00の発言を編集',exact:true}).click();
  const text=row.getByRole('textbox',{name:'00:00:00の発言',exact:true});await text.fill('再生中も未確定本文を保持します。');
  await play();await pause();await text.evaluate(e=>{window.monBrowserFixture.textNode=e;e.setSelectionRange(3,8);});
  await slider().fill('192000');await player().getByRole('combobox',{name:'速度',exact:true}).selectOption('2');
  check(await text.evaluate(e=>e===window.monBrowserFixture.textNode&&e.selectionStart===3&&e.selectionEnd===8)&&await calls('save_transcript_draft')===0,'playback confirmed or remounted pending text');
  await row.getByRole('button',{name:'キャンセル',exact:true}).click();results.push('seek and speed preserve unconfirmed individual text node and selection without a save');
  await boot();await page.evaluate(()=>window.monBrowserFixture.failPlayback=true);await row.getByRole('button',{name:'音声を再生',exact:true}).click();
  await page.locator('.toast-host').getByRole('alert').filter({hasText:'ハッシュ'}).waitFor();check(await calls('read_playback_chunk')===0,'failed hash validation played PCM');results.push('failed audio verification is a toast and never plays unverified PCM');
  await boot();await play();await nav.getByRole('button',{name:'設定',exact:true}).click();await dock.waitFor({state:'hidden'});await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='close_playback'));await open();check(await player().getByRole('button',{name:'再生',exact:true}).isVisible(),'return resumed old audio');results.push('settings exit closes the token and return never resumes it');
  await play();const before=await calls('close_playback');await nav.getByRole('button',{name:'会議一覧',exact:true}).click();await page.locator('.meeting-name').nth(1).click();await page.waitForFunction(()=>document.querySelector('.playback-meeting-title')?.textContent==='会議B');await page.waitForFunction(n=>window.monBrowserFixture.calls.filter(c=>c.command==='close_playback').length>n,before);check(await player().getByRole('button',{name:'再生',exact:true}).isVisible(),'meeting switch adopted old player');results.push('list navigation changes the owner and closes old audio without auto-playing');
  await boot();await row.getByRole('button',{name:'00:00:00の発言を編集',exact:true}).click();await text.fill('失敗時の本文を保持します。');await page.evaluate(()=>window.monBrowserFixture.failSave=true);await nav.getByRole('button',{name:'設定',exact:true}).click();await editor.getByRole('button',{name:'保存を再試行',exact:true}).waitFor();check(await text.inputValue()==='失敗時の本文を保持します。'&&await dock.isVisible(),'failed navigation lost input or owner');results.push('failed navigation saving retains the current meeting and pending body');
  await boot();await page.evaluate(()=>window.monBrowserFixture.holdPlayback=true);await row.getByRole('button',{name:'音声を再生',exact:true}).click();await page.waitForFunction(()=>typeof window.monBrowserFixture.releasePlayback==='function');await nav.getByRole('button',{name:'設定',exact:true}).click();await page.evaluate(()=>{window.monBrowserFixture.holdPlayback=false;window.monBrowserFixture.releasePlayback();});await page.waitForFunction(()=>window.monBrowserFixture.calls.some(c=>c.command==='close_playback'));await open();check(await player().getByRole('button',{name:'再生',exact:true}).isVisible(),'late response populated replacement dock');results.push('late preparation after leaving releases its token and cannot populate a replacement dock');
  return {passed:results.length,results};
}
