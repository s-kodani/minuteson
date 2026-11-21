async (page,capturePath) => {
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  const count=cmd=>page.evaluate(cmd=>window.__mon.plainCalls.filter(c=>c.cmd===cmd).length,cmd);
  const panel=()=>page.getByRole("region",{name:"議事録",exact:true});
  const input=()=>page.getByRole("textbox",{name:"議事録のMarkdown本文",exact:true});
  const dialog=()=>page.locator("dialog[open]");
  const meetings=()=>page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true});
  const open=async()=>{await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();await page.getByRole("tab",{name:"議事録",exact:true}).click();await panel().getByRole("button",{name:"議事録を編集",exact:true}).waitFor();};
  const generate=()=>panel().getByRole("button",{name:"議事録を生成",exact:true}).click();
  const dismiss=async()=>{await dialog().locator(":scope > .common-modal-dismiss-row > [data-dialog-cancel]").click();};
  await page.setViewportSize({width:1280,height:820});
  await page.getByRole("button",{name:"議事録UI確認",exact:true}).click();
  const tab = name => page.getByRole("tab", {name, exact:true});
  const reads = command => page.evaluate(command => window.__mon.plainReads.filter(cmd => cmd === command).length, command);
  await tab("文字起こし").waitFor();
  await page.clock.runFor(3100);
  check(await reads("get_minutes_markdown")===0 && await reads("get_minutes_source")===0, "opening transcription queried the unvisited minutes tab");
  await tab("議事録").click();
  await panel().getByRole("heading",{name:"点検会議",exact:true}).waitFor();
  check(await count("save_minutes_markdown")===0&&await count("start_markdown_minutes_generation")===0,"initial mount saved or generated");
  check(await panel().locator('[contenteditable],.minutes-primary-toolbar,.fixed-evidence,[data-evidence-id]').count()===0,"advanced editor/evidence survived production switch");
  results.push("product main window previews exact saved Markdown without implicit generation/save or rich evidence controls");
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();await input().fill("# 人間の編集\n\n- 点検日\n");
  await tab("文字起こし").click();
  const hiddenReads = await Promise.all([reads("get_minutes_markdown"), reads("get_minutes_source")]);
  await page.evaluate(() => { window.__mon.emitPlain("analysis-version-changed"); window.__mon.emitPlain("model-profile-changed"); });
  await page.clock.runFor(6100);
  check(await reads("get_minutes_markdown")===hiddenReads[0] && await reads("get_minutes_source")===hiddenReads[1], "a hidden minutes tab read on versions or polling");
  await tab("議事録").click();
  check(await input().inputValue()==="# 人間の編集\n\n- 点検日\n"&&await count("save_minutes_markdown")===0,"tab switch lost or autosaved human input");
  await page.waitForFunction(before => window.__mon.plainReads.filter(cmd => cmd === "get_minutes_markdown").length > before, hiddenReads[0]);
  results.push("the minutes tab skips reads before first visit and while hidden, verifies on reopen and preserves the draft without autosaving");
  await meetings().click();await dialog().getByRole("heading",{name:"議事録の未保存の編集"}).waitFor();
  await dialog().getByRole("button",{name:"確認をキャンセル",exact:true}).click();
  check(await input().inputValue().then(v=>v.includes("人間の編集"))&&await count("save_minutes_markdown")===0,"cancel navigated or saved");
  // The fixture can independently report unavailable playback audio. Only
  // navigation/edit operation errors diagnose cancellation of this dialog.
  check(await page.getByRole("alert").filter({hasText:/議事録操作|会議の操作/}).count()===0,"normal navigation cancellation displayed an error");
  await page.waitForFunction(() => document.activeElement === document.querySelector('textarea[aria-label="議事録のMarkdown本文"]'));
  results.push("navigation lock does not block save/discard/cancel modal; cancellation retains input");
  await meetings().click();await dialog().getByRole("button",{name:"編集を破棄して続ける",exact:true}).click();
  await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();check(await count("save_minutes_markdown")===0,"discard wrote unwanted input");await open();
  await panel().getByRole("heading",{name:"点検会議",exact:true}).waitFor();
  results.push("explicit discard returns to confirmed saved text without any save");
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();await input().fill("# 保存して移動\n");
  await meetings().click();await dialog().getByRole("button",{name:"保存して続ける",exact:true}).click();
  await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();check(await count("save_minutes_markdown")===1,"explicit leave-save wrote wrong number");await open();await panel().getByRole("heading",{name:"保存して移動",exact:true}).waitFor();
  results.push("explicit leave-save persists the exact plain body before navigation and reloads it on return");
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();await input().fill("# 保存失敗時の入力\n");await page.evaluate(()=>window.__mon.plainMode="failure");
  await meetings().click();await dialog().getByRole("button",{name:"保存して続ける",exact:true}).click();await dialog().getByRole("alert").filter({hasText:"処理を完了できませんでした"}).first().waitFor();await dismiss();
  check(await input().inputValue()==="# 保存失敗時の入力\n","failed navigation save lost input");
  await page.evaluate(()=>window.__mon.plainMode="success");await panel().getByRole("button",{name:"保存を再試行",exact:true}).click();await panel().getByRole("status").filter({hasText:/^保存済み$/}).waitFor();
  check(await page.evaluate(()=>{const c=window.__mon.plainCalls.filter(c=>c.cmd==="save_minutes_markdown");return JSON.stringify(c[1])===JSON.stringify(c[2]);}),"save retry changed operation/base/body");
  results.push("navigation save failure blocks departure; explicit retry preserves the identical operation and text");
  check(await panel().getByRole("button",{name:"保存",exact:true}).evaluate(button=>{
    const bounds=button.getBoundingClientRect();
    return button.contains(document.elementFromPoint(bounds.x+bounds.width/2,bounds.y+bounds.height/2));
  }),"retained notifications cover the next explicit save");
  await input().fill("# 応答喪失の保存\n");await page.evaluate(()=>window.__mon.plainMode="lost");await panel().getByRole("button",{name:"保存",exact:true}).click();await page.getByRole("alert").filter({hasText:"保存できませんでした"}).first().waitFor();
  await input().fill("# 応答待ち中の追記\n");await page.evaluate(()=>window.__mon.plainMode="success");await panel().getByRole("button",{name:"保存を再試行",exact:true}).click();await panel().getByRole("status").filter({hasText:"未保存"}).waitFor();
  check(await input().inputValue()==="# 応答待ち中の追記\n","lost success displaced later human input");await panel().getByRole("button",{name:"保存",exact:true}).click();await panel().getByRole("status").filter({hasText:/^保存済み$/}).waitFor();
  results.push("lost success recovery acknowledges original save while newer human input remains unsaved until explicit save");
  await generate();await dialog().getByRole("heading",{name:"議事録を再生成",exact:true}).waitFor();
  check((await dialog().innerText()).includes("旧版は保存しません"),"replacement consequences missing");await dialog().getByRole("button",{name:"確認をキャンセル",exact:true}).click();check(await count("start_markdown_minutes_generation")===0,"cancelled regeneration ran");
  await generate();await dialog().getByRole("heading",{name:"議事録を再生成",exact:true}).waitFor();await dialog().getByRole("button",{name:"議事録を再生成",exact:true}).click();
  await dialog().waitFor({state:"hidden"});
  await page.evaluate(()=>window.__mon.completePlain());await panel().getByRole("status").filter({hasText:/^保存済み$/}).waitFor();
  await panel().getByRole("button",{name:"議事録のプレビューを表示",exact:true}).click();await panel().getByRole("heading",{name:"生成された議事録",exact:true}).waitFor();
  check(await count("start_markdown_minutes_generation")===1,"regeneration duplicated admission");
  check(await page.evaluate(()=>{const call=window.__mon.plainCalls.find(c=>c.cmd==="start_markdown_minutes_generation");return call.args.expected.lockVersion===5&&call.args.request.input.expectedAnalysisId==="analysis";}),"generation did not freeze human target and current transcription");
  results.push("regeneration explicitly confirms replacement, enqueues Markdown worker once and previews result without candidate adoption");
  await generate();await dialog().getByRole("heading",{name:"議事録を再生成",exact:true}).waitFor();await dialog().getByRole("button",{name:"議事録を再生成",exact:true}).click();await dialog().waitFor({state:"hidden"});
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();await input().fill("# 生成中の人間の保存\n");await panel().getByRole("button",{name:"保存",exact:true}).click();await panel().getByRole("status").filter({hasText:/^保存済み$/}).waitFor();
  await page.evaluate(()=>window.__mon.completePlain());
  check(await input().inputValue()==="# 生成中の人間の保存\n"&&await page.evaluate(()=>window.__mon.jobs.at(-1).state==="failed"),"late generation displaced human save");
  results.push("late generation keeps human saved body instead of overwriting it");
  await page.evaluate(()=>{window.__mon.emitPlain("analysis-version-changed");window.__mon.emitPlain("model-profile-changed");});
  await panel().getByRole("button",{name:"議事録のプレビューを表示",exact:true}).click();await panel().getByRole("heading",{name:"生成中の人間の保存",exact:true}).waitFor();check(await count("start_markdown_minutes_generation")===2,"source/model hint triggered automatic regeneration");
  results.push("source and model changes refresh state, never automatically regenerate the human document");
  const failedId=await page.evaluate(()=>window.__mon.jobs.at(-1).id);
  await page.evaluate(()=>{window.__mon.pref.payload.standard.modelPath="/synthetic/current-global-after-failure.gguf";window.__mon.emitPlain("model-profile-changed");});
  check(await panel().getByRole("button",{name:"再試行",exact:true}).count()===0&&await panel().getByRole("button",{name:"議事録の操作",exact:true}).count()===0,"failed job retry/options remain");
  await generate();await dialog().getByRole("button",{name:"議事録を再生成",exact:true}).click();await dialog().waitFor({state:"hidden"});
  check(await page.evaluate(failedId=>{const data=window.__mon,newJob=data.jobs.at(-1),oldJob=data.jobs.find(job=>job.id===failedId),call=data.plainCalls.filter(c=>c.cmd==="start_markdown_minutes_generation").at(-1);return newJob.id!==failedId&&newJob.attempt===1&&newJob.retryOfJobId===null&&oldJob.state==="failed"&&call.args.request.modelPath==="/synthetic/current-global-after-failure.gguf"&&!data.plainCalls.some(c=>c.cmd==="retry_minutes_generation");},failedId),"generation after failure reused old job/model or mutated failure history");
  await page.evaluate(()=>window.__mon.completePlain());await panel().getByRole("heading",{name:"生成された議事録",exact:true}).waitFor();
  results.push("same generate button starts a fresh attempt with current global model after failure, without retrying old job");
  for(const theme of ["light","dark"])for(const width of [1280,360]){
    await page.setViewportSize({width,height:820});await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"plain production panel overflows horizontally");
    check(await panel().locator(".plain-minutes-document").evaluate(e=>{const paper=e.getBoundingClientRect(),body=e.parentElement.getBoundingClientRect();return Math.abs(paper.width-body.width)<1;}),"preview keeps old narrow paper cap");
    check(await panel().locator(".plain-minutes-primary-actions button").evaluateAll(buttons=>buttons.filter(b=>b.getClientRects().length).every(b=>b.getBoundingClientRect().height===32)),"neighboring plain minutes buttons have different heights");
    await page.screenshot({path:`plain-minutes-${theme}-${width}.png`});results.push(`plain production paper fits available width and buttons share 32px height: ${theme}/${width}`);
  }
  await page.setViewportSize({width:1280,height:820});await page.goto("http://127.0.0.1:1420/?empty=1");await open();
  await panel().getByRole("button",{name:"議事録を生成",exact:true}).click();await panel().getByRole("status").filter({hasText:"議事録生成"}).waitFor();
  check(await dialog().count()===0&&await count("start_markdown_minutes_generation")===1,"first generation used adoption/overwrite dialog or duplicated start");
  check(await page.evaluate(()=>window.__mon.plainCalls.find(c=>c.cmd==="start_markdown_minutes_generation").args.expected===null),"first generation invented a saved target");
  await page.evaluate(()=>window.__mon.completePlain());await panel().getByRole("heading",{name:"生成された議事録",exact:true}).waitFor();
  results.push("first generation fixes an absent target and immediately previews success without adoption");

  await page.goto("http://127.0.0.1:1420/?resilience=1");
  await page.getByText(/会議一覧を取得できませんでした/).waitFor();
  check(await page.getByText("保存済み会議はありません。",{exact:true}).count()===0,"failed meeting read rendered an empty library");
  await page.setViewportSize({ width: 1360, height: 900 });
  await page.evaluate(()=>{window.__mon.health.listError=false;window.__mon.minutesPrefReadFailure=true;window.__mon.health.diagnosticsReadFailure=true;});
  await page.getByRole("button",{name:"再読込",exact:true}).click();
  const unreadStatuses = page.getByText("確認が必要な会議があるか分かりませんでした。会議一覧の「再読込」を選んでください。", { exact: true });
  await unreadStatuses.waitFor();
  await page.screenshot({ path: capturePath("meeting-status-read-failed.png") });
  await page.evaluate(()=>{window.__mon.health.diagnosticsReadFailure=false;});
  await page.getByRole("button",{name:"再読込",exact:true}).click();
  await unreadStatuses.waitFor({ state: "hidden" });
  const affected=page.getByRole("row").filter({hasText:"議事録UI確認"});
  await affected.getByText("要確認",{exact:true}).waitFor();
  await affected.getByRole("button",{name:"議事録UI確認",exact:true}).click();
  await page.getByText(/確定した文字起こしを閲覧専用で表示/).waitFor();
  await page.locator(".utterance-list").getByText("点検日を確認する",{exact:true}).waitFor();
  check(await page.getByRole("button",{name:/発言を編集/}).count()===0,"unconfirmed draft read allowed editing confirmed transcript");
  const noticeLayout = tabPanel => tabPanel.locator(".meeting-document-notice").evaluate(notice => {
    const panel = notice.closest("[role=tabpanel]");
    const headingRow = panel.querySelector(".card-head");
    const heading = headingRow.querySelector("h2").getBoundingClientRect();
    const range = document.createRange();
    range.selectNodeContents(notice);
    const firstLine = range.getClientRects()[0];
    const { fontFamily, fontSize, fontWeight, lineHeight, color, paddingTop, paddingBottom } = getComputedStyle(notice);
    const bounds = notice.getBoundingClientRect();
    const following = panel.querySelector(".plain-minutes-body, .transcript-feedback button");
    const headingEnd = headingRow.getBoundingClientRect().bottom - parseFloat(getComputedStyle(headingRow).paddingBottom);
    return {
      left: firstLine.left - heading.left,
      top: firstLine.top - heading.bottom,
      font: [fontFamily, fontSize, fontWeight, lineHeight, color],
      above: bounds.top + parseFloat(paddingTop) - headingEnd,
      below: following.getBoundingClientRect().top - (bounds.bottom - parseFloat(paddingBottom)),
    };
  });
  for (const width of [1360, 640]) {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole("tab", { name: "文字起こし", exact: true }).click();
    const transcriptNotice = await noticeLayout(page.locator("#meeting-panel-transcript"));
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await panel().getByText(/生成設定を確認できませんでした/).waitFor();
    const minutesNotice = await noticeLayout(page.locator("#meeting-panel-minutes"));
    check(Math.abs(minutesNotice.left) <= 1 && Math.abs(transcriptNotice.left) <= 1, `notice text and heading do not share a left edge at ${width}px`);
    check(Math.abs(minutesNotice.top - transcriptNotice.top) <= 1, `notice text does not start in the same position below both headings at ${width}px (minutes: ${JSON.stringify(minutesNotice)}, transcript: ${JSON.stringify(transcriptNotice)})`);
    check(JSON.stringify(minutesNotice.font) === JSON.stringify(transcriptNotice.font), `generation and transcript notice styles differ at ${width}px`);
    const gap = width > 760 ? 12 : 8;
    for (const [name, notice] of [["minutes", minutesNotice], ["transcript", transcriptNotice]]) {
      check(Math.abs(notice.above - gap) <= 1 && Math.abs(notice.below - gap) <= 1, `${name} notice has ${notice.above}px above and ${notice.below}px below at ${width}px`);
    }
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  const modelNotice = page.locator(".mon-notification-stack [data-sonner-toast]").filter({ hasText: "モデル削除: 削除記録を確認できません" });
  if (await modelNotice.count()) {
    await modelNotice.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
    await modelNotice.waitFor({ state: "detached" });
  }
  await page.evaluate(() => { window.__mon.minutesInputsReadFailure = true; window.__mon.emitPlain("analysis-version-changed"); });
  await panel().getByText(/文字起こし、生成設定、生成の進み具合を確認できませんでした/).waitFor();
  await page.screenshot({ path: capturePath("minutes-generation-inputs-unavailable.png") });
  await page.evaluate(() => { window.__mon.minutesInputsReadFailure = false; window.__mon.emitPlain("analysis-version-changed"); });
  await page.getByRole("tab", { name: "文字起こし", exact: true }).click();
  const transcriptPanel = page.locator("#meeting-panel-transcript");
  await transcriptPanel.getByText(/確定した文字起こしを閲覧専用で表示/).waitFor();
  const transcriptReload = () => transcriptPanel.getByRole("button", { name: "文字起こしを再読込", exact: true });
  const checkAfterReload = async (following, expectedGap) => {
    const reloadBottom = await transcriptReload().evaluate(button => button.getBoundingClientRect().bottom);
    const nextTop = await following.evaluate(element => element.getBoundingClientRect().top + parseFloat(getComputedStyle(element).paddingTop));
    check(Math.abs(nextTop - reloadBottom - expectedGap) <= 1, `reload button and following content have ${nextTop - reloadBottom}px between them`);
  };
  await checkAfterReload(transcriptPanel.locator(".transcript-main"), 12);
  await page.screenshot({ path: capturePath("transcript-draft-read-unverified.png") });
  const previewText = "点検日を確認する。\n次回の点検項目を共有する。";
  for (const [mode, review, notice, filename] of [
    ["absent", null, "文字起こしの下書きを確認できません。閲覧できる確定本文はありません。編集中の内容は表示できません。", "transcript-draft-read-unverified-no-confirmed-body.png"],
    ["invalid", { text: previewText, complete: false }, "保存済みの文字起こしを確認できませんでした。読み取れた発言だけを閲覧専用で表示しています。", "transcript-evidence-invalid-review-only.png"],
    ["invalid", { text: previewText, complete: true }, "保存済みの文字起こしを確認できませんでした。読み取れた本文を閲覧専用で表示しています。", "transcript-evidence-invalid-complete-body.png"],
    ["invalid", null, "保存済みの文字起こしを確認できませんでした。", "transcript-evidence-invalid-no-body.png"],
  ]) {
    await page.evaluate(({ mode, review }) => {
      window.__mon.health.transcriptReadMode = mode;
      window.__mon.health.transcriptReview = review;
      window.__mon.emitPlain("analysis-version-changed");
    }, { mode, review });
    await transcriptPanel.getByText(notice, { exact: true }).waitFor();
    if (review) await checkAfterReload(transcriptPanel.getByLabel("確認用の文字起こし本文", { exact: true }), 12);
    await page.screenshot({ path: capturePath(filename) });
  }
  await page.setViewportSize({ width: 400, height: 900 });
  await page.evaluate(() => {
    const health = window.__mon.health;
    health.transcriptReview = { text: Array.from({ length: 100 }, (_, index) => `発言 ${index + 1}：点検項目を確認しました。`).join("\n"), complete: false };
    window.__mon.emitPlain("analysis-version-changed");
  });
  const reviewText = transcriptPanel.getByLabel("確認用の文字起こし本文", { exact: true });
  await reviewText.filter({ hasText: /発言 100：/ }).waitFor();
  await checkAfterReload(reviewText, 8);
  await page.screenshot({ path: capturePath("transcript-review-long-narrow.png") });
  await reviewText.focus();
  await page.keyboard.press("End");
  await page.waitForFunction(() => document.querySelector(".transcript-review-text").scrollTop > 0);
  await page.setViewportSize({ width: 1360, height: 900 });
  results.push("generation and transcript notices share their alignment, typography, and equal spacing above and below");
  await page.evaluate(()=>{
    window.__mon.health.transcriptReadMode=null;window.__mon.health.transcriptReview=null;
    window.__mon.health.draftReadFailure=false;window.__mon.health.draftDiscarded=true;
  });
  await page.getByRole("button",{name:"文字起こしを再読込",exact:true}).click();
  await page.getByRole("button",{name:"00:00:01の発言を編集",exact:true}).waitFor();
  await page.screenshot({ path: capturePath("transcript-after-invalid-draft-discard.png"), fullPage: true });
  await page.getByRole("button",{name:"00:00:01の発言を編集",exact:true}).click();
  check(await page.getByText(/文字起こしの下書きを確認できません/).count()===0,"an absent draft still displayed a draft warning");
  const utterance=page.locator('[data-utterance-id="u"]');
  await utterance.getByRole("textbox",{name:"00:00:01の発言",exact:true}).fill("次の点検日を確認しました。");
  await utterance.getByRole("button",{name:"保存",exact:true}).click();
  await utterance.getByRole("button",{name:"00:00:01の発言を編集",exact:true}).filter({hasText:"次の点検日を確認しました。"}).waitFor();
  check(await page.evaluate(()=>window.__mon.health.draftCreated&&window.__mon.td.id==="fresh-transcript-draft"&&window.__mon.td.payload.utterances[0]?.effective_text==="次の点検日を確認しました。"),"transcript save did not start a new draft after the native discard");
  await page.getByRole("tab",{name:"議事録",exact:true}).click();
  await panel().getByRole("heading",{name:"点検会議",exact:true}).waitFor();
  await panel().getByText(/生成設定を確認できませんでした/).waitFor();
  check(await panel().getByRole("button",{name:"議事録を生成",exact:true}).isDisabled(),"failed generation settings allowed generation");
  await panel().getByRole("button",{name:"議事録を編集",exact:true}).click();
  await input().fill(`${await input().inputValue()}\n次回の確認事項を追記します。\n`);
  await page.screenshot({ path: capturePath("minutes-settings-unavailable-editable.png") });
  await input().fill("# 補助データに障害があっても保存");
  await panel().getByRole("button",{name:"保存",exact:true}).click();
  await panel().getByRole("status").filter({hasText:/^保存済み$/}).waitFor();
  await page.evaluate(()=>{window.__mon.plainReadFailure=true;window.__mon.emitPlain("analysis-version-changed");});
  await panel().getByRole("region",{name:"確認用の議事録本文"}).getByRole("heading",{name:"補助データに障害があっても保存"}).waitFor();
  check(await panel().getByRole("button",{name:"議事録を編集",exact:true}).count()===0 && await count("save_minutes_markdown")===1,"evidence fallback enabled editing or saved implicitly");
  await page.screenshot({ path: capturePath("minutes-evidence-invalid-review-only.png") });
  await page.evaluate(()=>{window.__mon.health.minutesReviewUnavailable=true;window.__mon.emitPlain("analysis-version-changed");});
  await panel().getByRole("region",{name:"確認用の議事録本文"}).waitFor({ state: "hidden" });
  await panel().getByText("保存済みの議事録を確認できませんでした。", { exact: true }).waitFor();
  await page.screenshot({ path: capturePath("minutes-evidence-invalid-no-body.png") });
  results.push("deletion list, audio and preferences faults preserve document reads; a lost draft reply blocks edits while confirmed draft discard allows normal editing and saving");

  await page.setViewportSize({ width: 1360, height: 900 });
  await page.evaluate(()=>{
    window.__mon.plainReadFailure=false;window.__mon.minutesPrefReadFailure=false;window.__mon.health.modelListingFailure=false;
    window.__mon.emitPlain("analysis-version-changed");
    window.__mon.emitPlain("model-profile-changed");
  });
  await panel().getByRole("heading",{name:"補助データに障害があっても保存"}).waitFor();
  await page.getByRole("alert").filter({ hasText: "モデル削除: 削除記録を確認できません" }).waitFor({ state: "hidden" });
  await page.locator(".mon-notification-stack [data-sonner-toast]").first().waitFor({ state: "detached" });
  await page.evaluate(()=>{window.__mon.health.shared=true;});
  const notice = page.getByRole("region", { name: "保存先の警告", exact: true });
  const openSalvage = () => page.getByRole("button", { name: "データ復元", exact: true });
  const noticeBounds = async (warnings, reference) => page.evaluate(({ warnings, reference }) => {
    const ref = document.querySelector(reference)?.getBoundingClientRect();
    const banners = [...document.querySelectorAll(warnings)];
    return !!ref && ref.width > 0 && banners.length > 0 && banners.every(banner => {
      const rect = banner.getBoundingClientRect();
      return Math.abs(rect.left - ref.left) <= 1 && Math.abs(rect.right - ref.right) <= 1;
    });
  }, { warnings, reference });
  await notice.getByText("保存先は閲覧専用です。会議を閲覧できます。", { exact: true }).waitFor();
  check(await openSalvage().count() === 0, "an access fault offered partial salvage");
  const before = await page.evaluate(() => window.__mon.health.storageReadCount);
  const recheckStorage = notice.getByRole("button", { name: "保存先の状態を再確認", exact: true });
  await recheckStorage.click();
  await page.waitForFunction(before => window.__mon.health.storageReadCount > before, before);
  await page.waitForFunction(() => document.querySelector('[aria-label="保存先の警告"] button')?.disabled === false);
  for (const message of ["アクセス権限を戻してください。", "書き込みの制限が続いています。", "音声に確認が必要です"]) {
    check(await page.getByText(message).count() === 0, `removed notice still visible: ${message}`);
  }
  check(await page.getByRole("button",{name:"新しい録音",exact:true}).isDisabled(),"shared hold admitted recording");
  check(await page.evaluate(() => window.__mon.intervalCalls.every(call => call.cmd !== "get_meeting_audio_summary")), "hiding the audio recovery notice admitted audio access");
  const detailNotices = ".meeting-detail-content > .workspace-warning";
  const minutesHeading = ".meeting-detail-content .minutes-card > .card-head";
  const checkStorageNoticeSpacing = async (width, tab) => {
    const spacing = await notice.evaluate(banner => {
      const tabs = document.querySelector(".meeting-detail-navigation").getBoundingClientRect();
      const heading = banner.parentElement.querySelector(".workspace-tab-panel:not([hidden]) .card-head");
      const headingTop = heading.getBoundingClientRect().top + parseFloat(getComputedStyle(heading).paddingTop);
      const contentBounds = [...banner.querySelectorAll(":scope > p, :scope > button")].map(item => item.getBoundingClientRect());
      return {
        above: Math.min(...contentBounds.map(rect => rect.top)) - tabs.bottom,
        below: headingTop - Math.max(...contentBounds.map(rect => rect.bottom)),
      };
    });
    check(Math.abs(spacing.above - 8) <= 1 && Math.abs(spacing.below - spacing.above) <= 1,
      `storage notice has ${spacing.above}px above and ${spacing.below}px below in ${tab} at ${width}px`);
  };
  check(await page.locator(detailNotices).count() === 1, "the separate audio recovery notice still rendered");
  for (const width of [1360, 640, 400]) {
    await page.setViewportSize({ width, height: 900 });
    check(await noticeBounds(detailNotices, minutesHeading), `detail notices do not align with the heading at ${width}px`);
    await checkStorageNoticeSpacing(width, "minutes");
    await page.screenshot({ path: capturePath(`workspace-detail-notices-${width}.png`) });
  }
  await page.getByRole("tab", { name: "文字起こし", exact: true }).click();
  for (const width of [1360, 400]) {
    await page.setViewportSize({ width, height: 900 });
    await checkStorageNoticeSpacing(width, "transcript");
    await page.screenshot({ path: capturePath(`workspace-readonly-transcript-${width}.png`) });
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  await page.evaluate(() => {
    window.__mon.health.meetingIssue = true;
    window.__mon.emitPlain("background-cleanup-finished");
  });
  await page.getByText("一部の会議情報を確認できません", { exact: true }).waitFor();
  check(await page.getByText("音声に確認が必要です", { exact: true }).count() === 0, "a separate meeting diagnosis also exposed the hidden audio text");
  await meetings().click();
  await affected.getByRole("button",{name:"議事録UI確認",exact:true}).waitFor();
  for (const width of [1360, 640]) {
    await page.setViewportSize({ width, height: 900 });
    check(await noticeBounds(".content > .workspace-warning", ".content .page-heading"), `list notice does not align with the heading at ${width}px`);
    check(await noticeBounds(".content > .workspace-warning", ".content .meeting-table-frame"), `list notice does not align with the meeting table at ${width}px`);
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  await page.screenshot({ path: capturePath("workspace-temporary-write-hold.png") });
  await page.evaluate(() => {
    window.__mon.health.salvageAvailable = true;
    window.__mon.health.holdReason = "対象を確認できない削除記録があります。元のデータを保持し、保存先の書込みを停止しました。";
    window.__mon.health.meetingIssue = false;
    window.__mon.emitPlain("background-cleanup-finished");
  });
  await openSalvage().waitFor();
  await page.screenshot({ path: capturePath("workspace-shared-rescue-available.png") });
  await affected.getByRole("button", { name: "議事録UI確認", exact: true }).click();
  await page.getByText("一部の会議情報を確認できません", { exact: true }).waitFor({ state: "hidden" });
  for (const [tab, label] of [["transcript", "文字起こし"], ["minutes", "議事録"]]) {
    await page.getByRole("tab", { name: label, exact: true }).click();
    for (const width of [1360, 400]) {
      await page.setViewportSize({ width, height: 900 });
      await checkStorageNoticeSpacing(width, tab);
      await page.screenshot({ path: capturePath(`workspace-readonly-${tab}-restore-${width}.png`) });
    }
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  await openSalvage().click();
  const salvageDialog = page.getByRole("dialog", { name: "データ復元", exact: true });
  const switchToSalvage = salvageDialog.getByRole("button", { name: "再起動して復元に進む", exact: true });
  await switchToSalvage.waitFor();
  await page.screenshot({ path: capturePath("workspace-readonly-minutes-restore-dialog.png") });
  await page.evaluate(() => {
    window.__mon.health.salvageAvailable = false;
    window.__mon.health.holdReason = "保存先へのアクセス権限を確認できません。アクセス権限を戻してください。";
    [...document.querySelectorAll("dialog[open] button")].find(button => button.textContent.trim() === "再起動して復元に進む").click();
  });
  await switchToSalvage.waitFor({ state: "hidden" });
  check(await salvageDialog.getByText(/アクセス権限を戻してください/).count() === 0, "revoked salvage displayed the hidden access message");
  check(await page.evaluate(() => !window.__mon.calls.some(call => call.cmd === "finish_main_close" && call.args.salvage === true)), "stale partial salvage eligibility caused a restart");
  results.push("only verified storage failures offer partial salvage, a revoked permission cannot restart the app, and storage notices preserve horizontal and symmetric vertical spacing across detail tabs");

  await page.goto("http://127.0.0.1:1420/?resilience=1");
  await page.getByText(/会議一覧を取得できませんでした/).waitFor();
  await page.evaluate(() => {
    window.__mon.health.listError = false;
    window.__mon.health.modelListingFailure = false;
  });
  await page.getByRole("button", { name: "再読込", exact: true }).click();
  await open();
  await panel().getByRole("button", { name: "議事録を編集", exact: true }).click();
  const unsavedText = "# 点検会議\n\n次回の確認事項を追記します。\n";
  await input().fill(unsavedText);
  await page.evaluate(() => {
    window.__mon.plainReadFailure = true;
    window.__mon.health.minutesReviewUnavailable = true;
    window.__mon.emitPlain("analysis-version-changed");
  });
  await panel().getByText(/編集中の内容は残っています/).waitFor();
  check(await input().inputValue() === unsavedText, "a failed read changed unsaved minutes");
  await page.screenshot({ path: capturePath("minutes-evidence-fails-retains-unsaved-edit.png") });
  return results;
}
