async (page, capturePath = path => path) => {
  const results=[], check=(value,message)=>{if(!value)throw Error(message);};
  await page.getByRole("button",{name:"会議A",exact:true}).click();
  await page.getByRole("heading",{name:"発言一覧",exact:true}).waitFor();
  check(await page.getByRole("button",{name:/編集オプション|内容を確認|候補を採用|直前版|エンジン設定/}).count()===0,"obsolete editing/model/candidate actions remain");
  check(await page.locator(".transcript-workspace-tools,.speaker-merge-tools").count()===0,"removed options are only hidden");
  results.push("meeting has no candidate, per-meeting model or edit-options screens");
  const search = page.getByRole("searchbox", { name: "発言を検索" });
  for (const width of [1360, 400]) {
    await page.setViewportSize({ width, height: 900 });
    check(await page.locator(".transcript-main").evaluate(main => {
      const search = main.querySelector("input").getBoundingClientRect();
      const list = main.querySelector(".utterance-list").getBoundingClientRect();
      const row = main.querySelector(".utterance-row");
      const { top, bottom } = row.getBoundingClientRect();
      const meta = row.querySelector(".utterance-meta").getBoundingClientRect();
      const body = row.querySelector(".utterance-text").getBoundingClientRect();
      return Math.abs(list.top - search.bottom - 12) <= 1 &&
        Math.abs(body.top - meta.bottom - 8) <= 1 &&
        Math.abs(meta.top - top - (bottom - body.bottom)) <= 1;
    }), `search and utterance spacing is inconsistent at ${width}px`);
    await page.screenshot({ path: capturePath(`utterance-spacing-${width}.png`) });
    await search.fill("一致しない検索語");
    const empty = page.getByText("条件に一致する発言はありません。", { exact: true });
    await empty.waitFor();
    check(await empty.evaluate(message => {
      const search = message.parentElement.querySelector("input").getBoundingClientRect();
      const notice = message.getBoundingClientRect();
      return Math.abs(notice.top - search.bottom - 12) <= 1 && Math.abs(notice.left - search.left) <= 1;
    }), `empty search message is not directly below the search box at ${width}px`);
    await page.screenshot({ path: capturePath(`utterance-no-search-results-${width}.png`) });
    await search.fill("");
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  results.push("search spacing and the empty state match at desktop and compact widths");
  const body=()=>page.getByRole("button",{name:"00:00:00の発言を編集",exact:true});
  const editor=()=>page.getByRole("textbox",{name:"00:00:00の発言",exact:true});
  const save=()=>page.locator(".utterance-text-edit").getByRole("button",{name:"保存",exact:true});
  await body().click(); await editor().fill("個別に修正した本文"); await save().click(); await editor().waitFor({state:"hidden"});
  check(await page.evaluate(()=>window.monBrowserFixture.draft.payload.utterances[0].effective_text==="個別に修正した本文"),"text not saved");
  await page.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="00:00:00の発言を編集");
  check(await body().evaluate(e=>e===document.activeElement),"saved utterance lost keyboard focus");
  await page.keyboard.press("Enter"); await editor().fill("キャンセルする本文"); await page.keyboard.press("Escape"); await editor().waitFor({state:"hidden"});
  await page.waitForFunction(()=>document.activeElement?.getAttribute("aria-label")==="00:00:00の発言を編集");
  check(await body().evaluate(e=>e===document.activeElement) && await body().innerText()==="個別に修正した本文","Escape lost focus or changed saved text");
  results.push("one utterance edits and saves without opening an options screen");
  await page.getByRole("button",{name:"田中の名前を変更",exact:true}).click();
  const name=page.getByRole("dialog",{name:"話者名を変更",exact:true});
  await name.getByRole("textbox").fill("山田"); await name.getByRole("button",{name:"保存",exact:true}).click();await name.waitFor({state:"hidden"});
  check(await page.getByRole("button",{name:"山田の名前を変更",exact:true}).count()===1,"speaker name not reflected");
  results.push("speaker label opens a small rename modal and saves");
  await page.getByRole("button",{name:"00:00:00の話者を変更",exact:true}).click();
  const assignment=page.getByRole("dialog",{name:"00:00:00の話者を変更",exact:true});
  const assignmentWrites=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length);
  await assignment.getByRole("checkbox",{name:"佐藤",exact:true}).check();
  check(await assignment.isVisible() && await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length)===assignmentWrites,"checkbox saved or closed assignment");
  await assignment.getByRole("button",{name:"保存",exact:true}).click();
  await page.waitForFunction(()=>window.monBrowserFixture.draft.payload.utterances.find(u=>u.utterance_id==="speech-a").speaker_ids.includes("speaker-b"));
  await assignment.waitFor({state:"hidden"});
  results.push("individual speaker assignment saves without bulk selection");
  await body().click();await editor().fill("保存失敗でも保持する本文");
  await page.evaluate(()=>{window.monBrowserFixture.failSave=true;});await save().click();
  const retrySave = page.getByRole("button",{name:"保存を再試行",exact:true});
  await retrySave.waitFor();
  check(await retrySave.evaluate(button => {
    const heading = document.querySelector(".transcript-primary-heading h2").getBoundingClientRect();
    const previous = button.closest(".transcript-feedback").previousElementSibling.getBoundingClientRect();
    const bounds = button.getBoundingClientRect();
    return Math.abs(bounds.left - heading.left) <= 1 && bounds.top - previous.bottom >= 8 && bounds.width < 200;
  }), "retry button is flush with the edge or stretches across the tab");
  await page.screenshot({ path: capturePath("utterance-save-retry.png") });
  const failures=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length);
  await page.getByRole("button",{name:"設定",exact:true}).click();
  check(await editor().inputValue()==="保存失敗でも保持する本文","navigation lost failed input");
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length)===failures,"navigation replayed failed write");
  await page.evaluate(()=>{window.monBrowserFixture.failSave=false;});
  await page.getByRole("button",{name:"保存を再試行",exact:true}).click();await editor().waitFor({state:"hidden"});
  results.push("save failure retains input and blocks leaving/replay until explicit retry");
  await body().click();await editor().fill("応答喪失の保存結果");
  await page.evaluate(()=>{window.monBrowserFixture.lostTranscriptSaveResponse=true;}); await save().click();
  await page.getByRole("button",{name:"保存を再試行",exact:true}).waitFor();
  const lost=await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length);
  await page.getByRole("button",{name:"保存を再試行",exact:true}).click();await editor().waitFor({state:"hidden"});
  check(await page.evaluate(()=>window.monBrowserFixture.calls.filter(c=>c.command==="save_transcript_draft").length)===lost,"readback resent a successful write");
  results.push("lost save acknowledgement is proven by readback without a duplicate write");
  await body().click();await editor().fill("移動する前に保存");await page.getByRole("button",{name:"設定",exact:true}).click();
  await page.getByRole("heading",{name:"設定",exact:true}).waitFor();
  check(await page.evaluate(()=>window.monBrowserFixture.draft.payload.utterances.find(u=>u.utterance_id==="speech-a").effective_text==="移動する前に保存"),"leaving omitted pending text");
  await page.getByRole("button",{name:"前の画面へ戻る",exact:true}).click();await page.getByRole("heading",{name:"発言一覧",exact:true}).waitFor();
  check((await body().innerText())==="移動する前に保存","return lost persisted text");
  results.push("leaving and returning saves and reloads individual human editing");
  await page.screenshot({path:"simple-transcript.png"});
  return {passed:results.length,results};
}
