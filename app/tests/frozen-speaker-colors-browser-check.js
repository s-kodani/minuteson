// Production fixed-source readers. Synthetic IPC; no native application/audio.
async (page,capturePath,detailUI) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.reload();await page.waitForFunction(()=>!!window.__mon);
  await page.evaluate(()=>{
    const f=window.__mon;
    f.analysis.speakers=[{id:"same-id",label:"話者1",displayName:"現在の表示名"},{id:"unnamed",label:"話者2",displayName:null}];
    f.analysis.utterances[0].speakerIds=["same-id","unnamed"];
    f.td.payload.utterances[0].speaker_ids=["same-id","unnamed"];
    f.fixedSpeakers["fixed-a"]=[{id:"same-id",label:"話者1",displayName:"生成時の表示名"},{id:"unnamed",label:"固定話者2",displayName:null}];
    f.fixedSpeakers["fixed-p"]=[{id:"same-id",label:"話者1",displayName:"直前版の表示名"},{id:"unnamed",label:"直前話者2",displayName:null}];
  });
  await page.getByRole("button",{name:"開く",exact:true}).first().click();
  // The workspace deliberately keeps the inactive transcript mounted but
  // hidden. Compare its same-ID CSS without making it visible or switching
  // away from the fixed evidence whose DOM/read ownership is under test.
  const transcript=page.getByRole("region",{name:"文字起こし編集",exact:true,includeHidden:true});
  const current=transcript.locator('.assigned-speakers [data-speaker-id="same-id"]');await current.waitFor();
  const tone=await current.getAttribute("data-speaker-tone");
  check(await current.textContent()==="現在の表示名","current name fixture not displayed");
  await page.getByRole("tab",{name:"議事録",exact:true}).click();
  const doc=page.getByRole("region",{name:"議事録文書と編集",exact:true});
  await doc.locator(".minutes-document .evidence-chip").first().click();
  const group=doc.getByRole("complementary",{name:"項目の固定根拠一覧",exact:true});
  const source=group.getByRole("region",{name:"生成時に固定した文字起こし",exact:true});await source.waitFor();
  const frozen=source.locator('[data-speaker-id="same-id"]');
  check(await frozen.textContent()==="生成時の表示名"&&await frozen.getAttribute("data-speaker-tone")===tone&&await source.locator('.speaker-unset').count()===1,"evidence reused current name, remapped ID color or hid unconfirmed label");
  results.push("fixed evidence uses its frozen name and both assigned speakers; same meeting/speaker ID keeps its color without adopting the current name");
  const writes=()=>page.evaluate(()=>window.__mon.calls.filter(c=>["save_minutes_draft","save_transcript_draft","adopt_minutes_candidate","restore_minutes_version","start_minutes_generation"].includes(c.cmd)).length);
  const reads=()=>page.evaluate(()=>window.__mon.calls.filter(c=>c.cmd==="get_minutes_evidence_source").length);
  const original=await page.evaluate(()=>JSON.stringify({analysis:window.__mon.analysis,td:window.__mon.td,drafts:window.__mon.drafts,views:window.__mon.views,current:window.__mon.current}));
  const originalReads=await reads();await frozen.evaluate(e=>window.frozenSpeakerNode=e);
  const drawer=page.getByRole("dialog",{name:"議事録の目次・根拠",exact:true});
  const close=async()=>{if(await drawer.isVisible())await drawer.getByRole("button",{name:"議事録の目次・根拠を閉じる",exact:true}).click();};
  const open=async()=>{
    // Resize events commit asynchronously. Wait for the real right-side host
    // to adopt the viewport before deciding whether its drawer needs opening.
    await page.waitForFunction(()=>{const pane=document.querySelector('.workspace-panel-shell[data-side="right"]');return pane?.getAttribute("data-narrow")===String(matchMedia("(max-width: 1119px)").matches);});
    const trigger=page.getByRole("button",{name:"議事録の目次・根拠を開く",exact:true});
    if(await trigger.isVisible()&&await trigger.getAttribute("aria-expanded")!=="true")await trigger.click();
  };
  for(const width of [1280,960])for(const theme of ["light","dark"]){
    await close();await page.setViewportSize({width,height:820});
    await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
    await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);await open();
    await frozen.scrollIntoViewIfNeeded();
    const colors=await frozen.evaluate(e=>{const style=getComputedStyle(e);return [style.color,style.backgroundColor];});
    check(await current.evaluate((e,colors)=>{const s=getComputedStyle(e);return s.color===colors[0]&&s.backgroundColor===colors[1];},colors)&&await frozen.evaluate(e=>e===window.frozenSpeakerNode)&&await reads()===originalReads,"theme/drawer changed color consistency, frozen DOM or source read ownership");
    check(await frozen.evaluate(e=>{const r=e.getBoundingClientRect(),p=e.closest('.workspace-panel-body').getBoundingClientRect();return r.top>=p.top&&r.bottom<=p.bottom&&r.right<=innerWidth&&document.documentElement.scrollWidth<=innerWidth;}),"frozen name clipped or overflowed");
    await page.screenshot({path:`frozen-speaker-${theme}-${width}.png`});
    results.push(`frozen/current ID colors agree at ${width}px/${theme}; drawer/theme preserve the frozen node and read identity`);
  }
  await group.getByRole("button",{name:"固定文字起こし全文へ移動",exact:true}).click();
  const full=page.getByRole("region",{name:"固定文字起こし全文",exact:true});
  const target=full.getByRole("article",{name:"固定発言68・根拠の参照先",exact:true});await target.waitFor();
  check(await target.locator('[data-speaker-id="same-id"]').textContent()==="生成時の表示名"&&await target.locator('[data-speaker-id="same-id"]').getAttribute("data-speaker-tone")===tone&&await target.locator('.speaker-unset').count()===1,"fixed full did not share frozen source semantics");
  await target.scrollIntoViewIfNeeded();await page.screenshot({path:"fixed-full-speaker.png"});
  await full.getByRole("button",{name:"次の50発言",exact:true}).click();await full.getByRole("article",{name:"固定発言101",exact:true}).waitFor();
  check(await full.getByRole("article",{name:"固定発言101",exact:true}).locator('[data-speaker-id="same-id"]').getAttribute("data-speaker-tone")===tone,"fixed pagination reassigned speaker tone");
  await full.getByRole("button",{name:"議事録の元の位置へ戻る",exact:true}).click();await source.waitFor();
  check(await frozen.textContent()==="生成時の表示名"&&await reads()===originalReads&&await writes()===0,"fixed full return modified/reloaded evidence or published a write");
  results.push("fixed-full source and pagination retain name/ID color and unconfirmed text; return neither reloads fixed evidence nor writes document/transcript state");
  await close();await detailUI.openMinutes();await page.getByRole("button",{name:"議事録直前版を確認・復元",exact:true}).click();
  const comparison=page.getByRole("dialog",{name:"議事録直前版の復元確認 #1",exact:true});await comparison.waitFor();
  const previous=comparison.getByRole("region",{name:"復元する議事録（保存済み編集を含む）",exact:true});await previous.locator(".evidence-chip").first().click();
  const old=previous.getByRole("region",{name:"生成時に固定した文字起こし",exact:true});await old.waitFor();
  check(await old.locator('[data-speaker-id="same-id"]').textContent()==="直前版の表示名"&&await old.locator('[data-speaker-id="same-id"]').getAttribute("data-speaker-tone")===tone&&await old.locator('.speaker-unset').count()===1,"previous input inherited current/frozen-current names or lost stable ID tone");
  await old.scrollIntoViewIfNeeded();
  await page.screenshot({path:"previous-frozen-speaker.png"});await page.keyboard.press("Escape");
  check(await writes()===0&&await page.evaluate(original=>JSON.stringify({analysis:window.__mon.analysis,td:window.__mon.td,drafts:window.__mon.drafts,views:window.__mon.views,current:window.__mon.current})===original,original),"read-only names/colors mutated fixed/current data");
  results.push("previous-version comparison preserves its own frozen name and label while keeping the same ID tone, with no data/publication writes");
  return {passed:results.length,results};
}
