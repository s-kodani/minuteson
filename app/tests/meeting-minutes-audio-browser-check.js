// Full-meeting and frozen minutes playback through production UI; silent IPC.
async (page) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const dock=()=>page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true});
  const whole=()=>page.getByRole("group",{name:"会議全体の音声再生",exact:true});
  const fixed=()=>page.getByRole("group",{name:"発言の音声再生",exact:true});
  const document=()=>page.getByRole("region",{name:"議事録文書と編集",exact:true});
  const references=()=>document().getByRole("complementary",{name:"項目の固定根拠一覧",exact:true});
  const drawer=()=>page.getByRole("dialog",{name:"議事録の目次・根拠",exact:true});
  const token=()=>page.evaluate(()=>Object.keys(window.__mon.playbacks)[0]);
  const gone=id=>page.waitForFunction(id=>!window.__mon.playbacks[id],id);
  const writes=()=>page.evaluate(()=>window.__mon.calls.filter(c=>["save_minutes_draft","adopt_minutes_candidate","restore_minutes_version"].includes(c.cmd)).length);
  const boot=async(width=1200)=>{
    await page.setViewportSize({width,height:820});await page.reload();await page.waitForFunction(()=>!!window.__mon);
    await page.locator(".meeting-name").first().click();
    await page.getByRole("tab",{name:"議事録",exact:true}).click();await document().waitFor();
  };
  const pause=async(player)=>{await player.getByRole("button",{name:"一時停止",exact:true}).waitFor();await player.getByRole("button",{name:"一時停止",exact:true}).click();};
  const playWhole=async()=>{await dock().getByRole("button",{name:"会議全体を再生",exact:true}).click();await pause(whole());};
  const select=async()=>{await document().locator(".minutes-document .evidence-chip").first().click();await references().getByRole("region",{name:"生成時に固定した文字起こし",exact:true}).first().waitFor();};
  const evidence=()=>references().getByRole("article",{name:"固定根拠1",exact:true});
  await boot();const original=await page.evaluate(()=>JSON.stringify(window.__mon.drafts.a));
  await playWhole();const wholeToken=await token();await whole().evaluate(e=>{window.fullMeetingControls=e;});
  await whole().getByRole("slider",{name:"会議全体の再生位置",exact:true}).fill("4800000");
  await whole().getByRole("combobox",{name:"速度",exact:true}).selectOption("1.5");
  await page.getByRole("tab",{name:"文字起こし",exact:true}).click();await page.getByRole("tab",{name:"議事録",exact:true}).click();
  check(await whole().evaluate(e=>e===window.fullMeetingControls)&&await token()===wholeToken&&await whole().getByRole("slider",{name:"会議全体の再生位置",exact:true}).inputValue()==="4800000"&&await whole().getByRole("combobox",{name:"速度",exact:true}).inputValue()==="1.5","tab round trip replaced full meeting controls/token or position/rate");
  await select();await evidence().getByRole("button",{name:"根拠の音声を再生",exact:true}).click();await pause(fixed());await gone(wholeToken);
  check(await whole().count()===0&&await dock().getByRole("group",{name:"発言の音声再生",exact:true}).count()===1,"fixed evidence did not exclusively replace full meeting audio");
  check(await page.evaluate(()=>{const r=window.__mon.calls.find(c=>c.cmd==="prepare_minutes_evidence_playback").args.request;return r.minutesId==="a"&&r.expectedContentHash==="h-a"&&r.expectedInputContentHash==="ih-fixed-a"&&r.expectedEvidence.id==="e-a"&&r.expectedEvidence.analysisRevisionId==="fixed-a"&&r.expectedEvidence.startUs===1000000&&r.expectedEvidence.endUs===3000000;}),"full meeting controls weakened frozen evidence identity or interval");
  const fixedToken=await token();await playWhole();await gone(fixedToken);
  check(await fixed().count()===0&&await whole().count()===1&&await writes()===0&&await page.evaluate(o=>JSON.stringify(window.__mon.drafts.a)===o,original),"return to full meeting duplicated player or changed canonical minutes");
  results.push("full meeting playback survives tab round trip with the same paused position/rate; frozen evidence and full meeting replace each other exclusively without document writes");

  await boot(960);await document().getByRole("button",{name:"議事録を編集",exact:true}).click();
  const field=document().getByRole("textbox",{name:"議事録ブロック1",exact:true});
  await page.evaluate(()=>{window.__mon.failMinutesSave=true;});await field.fill("音声を切り替えても残る未保存の見出し");await field.evaluate(e=>{window.stagedMinutesField=e;});
  await document().getByRole("status").filter({hasText:"議事録の保存要確認"}).waitFor();
  const failedWrites=await writes();await playWhole();const drawerWhole=await token();await select();await drawer().waitFor();
  await evidence().getByRole("button",{name:"根拠の音声を再生",exact:true}).click();await pause(drawer().getByRole("group",{name:"発言の音声再生",exact:true}));await gone(drawerWhole);
  check(await whole().count()===0&&await dock().getByRole("group",{name:"発言の音声再生",exact:true}).count()===0,"drawer left full meeting playing or placed fixed controls in inert dock");
  const drawerFixed=await token();await drawer().getByRole("button",{name:"議事録の目次・根拠を閉じる",exact:true}).click();await playWhole();await gone(drawerFixed);
  check(await field.evaluate(e=>e===window.stagedMinutesField)&&await field.textContent()==="音声を切り替えても残る未保存の見出し"&&await writes()===failedWrites&&await page.evaluate(()=>window.__mon.drafts.a.payload.blocks[0].content[0].text==="点検会議"),"drawer playback switch remounted/discarded unsaved rich text or retried failed save");
  results.push("narrow drawer takes operable frozen controls from full meeting audio and releases them on explicit full replay while preserving the same failed-save rich field without new writes");

  await boot();await playWhole();const beforeFull=await token();await select();
  await evidence().getByRole("button",{name:"固定文字起こし全文へ移動",exact:true}).click();
  const full=page.getByRole("region",{name:"固定文字起こし全文",exact:true});await full.getByRole("article",{name:"固定発言68・根拠の参照先",exact:true}).waitFor();
  await full.getByRole("button",{name:"固定発言68の音声を再生",exact:true}).click();await pause(fixed());await gone(beforeFull);
  check(await page.evaluate(()=>{const r=window.__mon.calls.find(c=>c.cmd==="prepare_minutes_transcript_playback").args.request;return r.reference.minutesId==="a"&&r.reference.expectedInputContentHash==="ih-fixed-a"&&r.reference.expectedEvidence.id==="e-a"&&r.utteranceId==="u-fixed-a"&&r.expectedStartUs===1000000&&r.expectedEndUs===3000000;})&&await whole().count()===0,"fixed full playback lost frozen utterance/interval or retained full meeting audio");
  const fullToken=await token();await playWhole();await gone(fullToken);await full.getByRole("button",{name:"議事録の元の位置へ戻る",exact:true}).click();
  check(await whole().count()===1&&await fixed().count()===0&&await writes()===0&&await page.evaluate(()=>!window.__mon.calls.some(c=>["start_recording","stop_recording","start_minutes_generation"].includes(c.cmd))),"fixed-full return revived previous audio, wrote document or invoked recording/generation");
  results.push("fixed full-transcript audio replaces full meeting audio using its exact frozen utterance/interval; explicit full replay and document return never revive the released source or write minutes");
  return {passed:results.length,results};
}
