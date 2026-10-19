// Production React controls with synthetic IPC/CAS. No native app or recording.
async (page, capturePath, detailUI) => {
  const results = [],check = (v,m) => { if(!v) throw Error(m); };
  const editor = page.getByRole("region",{ name: "文字起こし編集",exact: true }), dialog = () => page.getByRole("dialog",{ name: "文字起こしの保存競合を比較",exact: true });
  const row = () => editor.getByRole("article",{ name: "00:00:00の発言行",exact: true }), input = () => row().getByRole("textbox",{ name: "00:00:00の発言",exact: true });
  const name = () => editor.getByRole("textbox",{ name: "話者A",exact: true });
  const counts = () => page.evaluate(() => { const f = window.monBrowserFixture; return [f.calls.filter(c => c.command === "save_transcript_draft").length,f.recovery.calls.filter(c => c.command === "save_transcript_edit_recovery").length]; });
  const advance = () => page.evaluate(() => {
    const f = window.monBrowserFixture; f.failSave = false; f.draft.lockVersion = 2;
    f.draft.payload = { schema_version:4,speaker_names: [{ speaker_id: "speaker-a",display_name: "最新確定名" }],utterances: [{ utterance_id: "speech-a",effective_text: "別操作の最新本文",speaker_ids: ["speaker-b"] }] };
  });
  const openMeeting = async () => { await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled); await page.locator(".meeting-name").first().click(); await detailUI.openTranscriptTools(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).waitFor(); };
  const boot = async () => {
    await page.reload(); await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click();
    await row().getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).click(); await input().fill("旧確定要求");
    await page.waitForFunction(() => { const f = window.monBrowserFixture; return f.recovery.records[f.draft.id]?.payload.buffers[0]?.value === "旧確定要求"; });
    await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
    await row().getByRole("button",{ name: "確定",exact: true }).click(); await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).waitFor();
    await input().fill("現在の未確定本文"); await name().fill("現在の未確定名"); await advance();
  };
  const compare = async () => { await editor.getByRole("button",{ name: "最新保存版と入力を比較",exact: true }).click(); await dialog().waitFor(); };
  const choose = async (body = "local",text = "local",speaker = "local") => {
    await dialog().getByRole("group",{ name: "本文 speech-a",exact: true }).getByRole("combobox",{ name: "残す変更",exact: true }).selectOption(body);
    await dialog().getByRole("group",{ name: "未確定本文 speech-a",exact: true }).getByRole("combobox",{ name: "再開する入力",exact: true }).selectOption(text);
    await dialog().getByRole("group",{ name: "未確定話者名 speaker-a",exact: true }).getByRole("combobox",{ name: "再開する入力",exact: true }).selectOption(speaker);
  };
  const accept = () => dialog().getByRole("button",{ name: "選択した内容で編集を再開",exact: true }).click();

  await boot(); const initial = await counts(); await compare(); await page.waitForTimeout(1000);
  check(JSON.stringify(initial) === JSON.stringify(await counts()),"comparison issued writes");
  check(await dialog().getByRole("button",{ name: "選択した内容で編集を再開",exact: true }).isDisabled(),"unselected input could resume");
  await dialog().getByText("結果未確認の本文保存要求",{ exact: true }).click();
  check(await dialog().getByRole("textbox",{ name: "元の本文保存要求",exact: true }).inputValue().then(v => v.includes("旧確定要求")),"frozen old request missing");
  results.push("read-only comparison exposes old request, new local input and all durable input; no autosave while open");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  check(await input().inputValue() === "現在の未確定本文" && await name().inputValue() === "現在の未確定名","closing lost input");
  check(await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).isVisible(),"closing retired pending request");
  results.push("Escape closes comparison while keeping original inputs and pending body request");
  await page.getByRole("banner").getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption("light");
  await page.waitForFunction(() => document.documentElement.dataset.theme === "light");
  await compare(); await choose(); await page.screenshot({ path: "conflict-light-1200.png",fullPage: false });
  await accept(); await dialog().waitFor({ state: "hidden" }); await page.waitForTimeout(1000);
  check(JSON.stringify(initial) === JSON.stringify(await counts()),"resume published body or journal");
  check(await input().inputValue() === "現在の未確定本文" && await name().inputValue() === "現在の未確定名","chosen local input missing");
  check(await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).count() === 0,"obsolete body request remained");
  results.push("explicit local resume releases old request without writes and keeps both autosave timers paused");
  await row().getByRole("button",{ name: "確定",exact: true }).click(); await input().waitFor({ state: "hidden" });
  check(await page.evaluate(() => { const p = window.monBrowserFixture.draft.payload; return p.utterances[0].effective_text === "現在の未確定本文" && p.utterances[0].speaker_ids.join() === "speaker-b" && p.speaker_names[0].display_name === "最新確定名"; }),"explicit confirm lost latest assignment or published other name");
  results.push("later explicit text confirmation uses new CAS and preserves latest assignment and unconfirmed name");
  await editor.getByRole("group",{ name: "話者Aの名前編集",exact: true }).getByRole("button",{ name: "キャンセル",exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.draft.payload.speaker_names[0]?.display_name === "最新確定名");
  check(await name().inputValue() === "最新確定名","rebased name cancel used old baseline");
  results.push("name cancellation after rebase restores the selected working baseline, not stale original name");

  await boot(); await compare(); await choose("latest","stored","omit"); const beforeStored = await counts(); await accept(); await dialog().waitFor({ state: "hidden" });
  check(await input().inputValue() === "旧確定要求" && await name().inputValue() === "最新確定名","stored choice or omitted name wrong");
  check(JSON.stringify(beforeStored) === JSON.stringify(await counts()),"stored resume changed record");
  await row().getByRole("button",{ name: "キャンセル",exact: true }).click(); await input().waitFor({ state: "hidden" });
  check(await row().getByLabel("00:00:00の発言本文",{ exact: true }).textContent() === "別操作の最新本文","cancel restored stale stored baseline");
  results.push("stored text and explicit omission resume read-only; Esc/cancel returns to latest selected body");

  await boot(); await compare(); await choose(); await page.evaluate(() => { window.monBrowserFixture.draft.lockVersion++; }); await accept();
  await dialog().getByRole("alert").waitFor(); check(await dialog().isVisible() && (await counts())[0] === 1,"changed body was overwritten");
  results.push("body version changing after preview rejects resume and retains old request");
  await dialog().getByRole("button",{ name: "最新の内容を比較し直す",exact: true }).click(); await page.waitForFunction(() => document.querySelector('dialog[open]')?.textContent.includes("最新保存版 3"));
  check(await dialog().getByRole("button",{ name: "選択した内容で編集を再開",exact: true }).isDisabled(),"refresh reused old choices");
  results.push("fresh comparison recaptures body and recovery and clears old selection proofs");

  await boot(); await compare(); await choose(); await page.evaluate(async () => {
    const f = window.monBrowserFixture,r = f.recovery.records[f.draft.id]; r.payload.buffers[0].value = "他操作の新しい回復入力"; r.lockVersion++; r.operationId = crypto.randomUUID(); r.contentHash = await (await import("/tests/transcript-recovery-browser-fixture.js")).fixtureHash(r.payload);
  }); await accept(); await dialog().getByRole("alert").waitFor();
  check(await page.evaluate(() => { const f = window.monBrowserFixture; return f.recovery.records[f.draft.id].payload.buffers[0].value === "他操作の新しい回復入力"; }),"changed journal was lost");
  results.push("changed durable recovery after preview blocks partial rebase and preserves newer stored input");
  await boot(); await compare(); await choose(); await page.evaluate(() => { const f = window.monBrowserFixture; delete f.recovery.records[f.draft.id]; }); await accept(); await dialog().getByRole("alert").waitFor();
  check((await counts())[0] === 1,"missing journal caused publish"); results.push("disappeared durable record rejects the compared proof without writing");
  await boot(); await compare(); await choose(); await page.evaluate(() => { window.monBrowserFixture.failDraftRead = true; }); await accept(); await dialog().getByRole("alert").waitFor();
  check((await counts())[0] === 1,"read failure caused replay"); results.push("failed final body read retains comparison and original request without replay");
  await boot(); await page.evaluate(() => { window.monBrowserFixture.currentId = "candidate"; }); await editor.getByRole("button",{ name: "最新保存版と入力を比較",exact: true }).click();
  await editor.getByRole("alert").filter({ hasText: "解析版が変わりました" }).waitFor(); check(await dialog().count() === 0 && await input().inputValue() === "現在の未確定本文","different analysis replaced input");
  results.push("changed analysis refuses comparison and never activates an old/new analysis automatically");
  await boot(); await page.evaluate(() => { window.monBrowserFixture.draft.id = crypto.randomUUID(); }); await editor.getByRole("button",{ name: "最新保存版と入力を比較",exact: true }).click();
  await editor.getByRole("alert").filter({ hasText: "同じ下書き" }).waitFor(); check(await dialog().count() === 0,"foreign draft resumable");
  results.push("a different active draft cannot retire the original body's pending request");

  await boot(); await page.evaluate(() => window.monBrowserFixture.crashSeed()); await page.reload();
  await page.waitForFunction(() => !!window.monBrowserFixture); await page.locator(".meeting-name").first().click(); await detailUI.openTranscriptTools(); await editor.getByRole("button",{ name: "未確定入力を確認",exact: true }).waitFor(); await compare();
  check(await dialog().getByRole("group",{ name: "未確定本文 speech-a",exact: true }).getByRole("combobox",{ name: "再開する入力",exact: true }).locator('option[value="local"]').getAttribute("disabled") !== null,"stored-only local side offered");
  await dialog().getByRole("group",{ name: "未確定本文 speech-a",exact: true }).getByRole("combobox",{ name: "再開する入力",exact: true }).selectOption("stored"); const beforeReloadResume = await counts(); await accept(); await dialog().waitFor({ state: "hidden" });
  check(await input().inputValue() === "旧確定要求" && JSON.stringify(beforeReloadResume) === JSON.stringify(await counts()),"stale durable restart could not resume without write");
  results.push("reloaded latest body can explicitly resume a stale stored-only buffer without old-version adoption");

  await boot(); await page.evaluate(() => { const f = window.monBrowserFixture; f.draft.payload.schema_version = 4; f.draft.payload.speaker_merges = [{ source_id: "speaker-a",target_id: "speaker-b" }]; }); await compare(); await choose(); await accept();
  await dialog().getByRole("alert").filter({ hasText: "選択した話者" }).waitFor(); check((await counts())[0] === 1,"hidden name partially resumed other text");
  await dialog().getByRole("group",{ name: "未確定話者名 speaker-a",exact: true }).getByRole("combobox",{ name: "再開する入力",exact: true }).selectOption("omit"); await accept(); await dialog().waitFor({ state: "hidden" });
  check(await input().inputValue() === "現在の未確定本文" && await name().count() === 0,"explicit hidden omission failed");
  results.push("hidden merged name blocks all resume until explicitly omitted; valid text remains available");

  await boot(); await name().fill("手元の確定待ち名");
  await editor.getByRole("group",{ name: "話者Aの名前編集",exact: true }).getByRole("button",{ name: "確定",exact: true }).click(); await name().fill("さらに新しい未確定名");
  await compare(); await choose();
  check(await dialog().getByRole("button",{ name: "選択した内容で編集を再開",exact: true }).isDisabled(),"same-field name conflict selected automatically");
  await dialog().getByRole("group",{ name: "話者名 speaker-a",exact: true }).getByRole("combobox",{ name: "残す変更",exact: true }).selectOption("latest");
  await accept(); await dialog().waitFor({ state: "hidden" });
  check(await name().inputValue() === "さらに新しい未確定名" && await page.evaluate(() => window.monBrowserFixture.draft.payload.speaker_names[0].display_name === "最新確定名"),"name conflict or newer raw input lost");
  results.push("same-field queued name conflict requires its own explicit choice and preserves newer unconfirmed name");

  await page.reload(); await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click();
  await row().getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).click();
  await page.evaluate(() => { window.monBrowserFixture.recovery.failSave = true; }); await input().fill("回復保存だけ未確認の本文");
  await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).waitFor(); await name().fill("回復保存だけ未確認の名"); await advance();
  const journalOnlyCounts = await counts(); await compare(); await choose("latest","local","local"); await accept(); await dialog().waitFor({ state: "hidden" }); await page.waitForTimeout(950);
  check(await input().inputValue() === "回復保存だけ未確認の本文" && await name().inputValue() === "回復保存だけ未確認の名" && JSON.stringify(journalOnlyCounts) === JSON.stringify(await counts()),"journal-only conflict replayed or lost input");
  check(await editor.getByRole("button",{ name: "回復保存の結果を確認",exact: true }).count() === 0,"obsolete journal-only request not retired");
  results.push("journal-only pending CAS can explicitly rebase without replay, publishing or losing either input type");

  await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({ length: 12 },(_,i) => ({ id: `speech-${i}`,startUs: i * 1_000_000,endUs: (i + 1) * 1_000_000,recognizedText: `原文${i}`,effectiveText: `原文${i}`,attributionState: "assigned",speakerIds: ["speaker-a"] }))));
  await openMeeting(); await editor.getByRole("button",{ name: "編集を開始",exact: true }).click();
  for(let i = 0; i < 12; i++) { const at = `00:00:${String(i).padStart(2,"0")}`, r = editor.getByRole("article",{ name: `${at}の発言行`,exact: true }); await r.getByRole("button",{ name: `${at}の発言を編集`,exact: true }).click(); await r.getByRole("textbox",{ name: `${at}の発言`,exact: true }).fill(`入力${i}`); }
  await page.waitForFunction(() => { const f = window.monBrowserFixture; return f.recovery.records[f.draft.id]?.payload.buffers.length === 12; });
  await page.evaluate(() => { window.monBrowserFixture.draft.lockVersion++; }); await compare();
  check(await dialog().getByRole("group").filter({ has: page.getByRole("combobox",{ name: "再開する入力",exact: true }) }).count() === 10,"preview page not bounded");
  for(const select of await dialog().getByRole("combobox",{ name: "再開する入力",exact: true }).all()) await select.selectOption("local");
  check(await dialog().getByRole("button",{ name: "選択した内容で編集を再開",exact: true }).isDisabled(),"first page selection partially resumed hidden input");
  await dialog().getByRole("button",{ name: "次の比較項目",exact: true }).click();
  check(await dialog().getByRole("combobox",{ name: "再開する入力",exact: true }).count() === 2,"last page input lost");
  for(const select of await dialog().getByRole("combobox",{ name: "再開する入力",exact: true }).all()) await select.selectOption("stored");
  await accept(); await dialog().waitFor({ state: "hidden" });
  check(await editor.getByRole("textbox",{ name: /の発言$/ }).count() === 12 && (await counts())[0] === 0,"paged choices lost raw buffers or published body");
  results.push("twelve inputs require choices across bounded ten-row pages and resume together without publication");

  await boot(); await page.getByRole("banner").getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption("dark");
  await page.waitForFunction(() => document.documentElement.dataset.theme === "dark");
  await compare(); await choose(); await page.setViewportSize({ width: 960,height: 900 });
  await page.screenshot({ path: "conflict-dark-960.png",fullPage: false });
  await page.setViewportSize({ width: 360,height: 800 }); await page.screenshot({ path: "conflict-dark-360.png",fullPage: false });
  check(await dialog().evaluate(el => el.scrollWidth <= el.clientWidth + 1),"narrow comparison has horizontal overflow");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  check(await editor.getByRole("button",{ name: "最新保存版と入力を比較",exact: true }).evaluate(el => el === document.activeElement),"dialog did not restore comparison opener");
  results.push("light/dark and 360px comparison remains readable without overflow and restores focus on close");
  return { passed: results.length,results };
}
