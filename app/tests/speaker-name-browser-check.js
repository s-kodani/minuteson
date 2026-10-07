// Actual React name controls, synthetic CAS/IPC only. No native data or audio.
async (page, capturePath, detailUI) => {
  const results = [], check = (ok,message) => { if(!ok) throw Error(message); };
  const editor = page.getByRole("region",{ name: "文字起こし編集",exact: true });
  const group = label => editor.getByRole("group",{ name: `${label}の名前編集`,exact: true });
  const name = label => group(label).getByRole("textbox",{ name: label,exact: true });
  const commit = label => group(label).getByRole("button",{ name: "確定",exact: true }).click();
  const cancel = label => group(label).getByRole("button",{ name: "キャンセル",exact: true }).click();
  const writes = () => page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "save_transcript_draft"));
  const payload = () => page.evaluate(() => window.monBrowserFixture.draft.payload);
  const field = editor.getByRole("textbox",{ name: "00:00:00の発言",exact: true });
  const waitName = (id,value) => page.waitForFunction(([id,value]) => window.monBrowserFixture.draft.payload.speaker_names.some(item => item.speaker_id === id && item.display_name === value),[id,value]);
  const boot = async () => {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.waitForFunction(() => !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    await page.getByRole("button",{ name: "開く",exact: true }).first().click(); await detailUI.openTranscriptTools();
    await editor.getByRole("button",{ name: "編集を開始",exact: true }).click(); await name("話者A").waitFor();
  };
  await boot(); await name("話者A").fill("入力中の名前"); await page.waitForTimeout(950);
  check((await writes()).length === 0 && (await payload()).speaker_names.length === 0 && await editor.getByRole("checkbox",{ name: "田中",exact: true }).first().isVisible() && await editor.getByText("未保存",{ exact: true }).isVisible(),"typing published name or changed speaker labels");
  results.push("idle name typing stays unpublished and labels retain their confirmed names");
  await name("話者B").fill("別の未確定名"); await editor.getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).click(); await field.fill("別の未確定本文"); await commit("話者A"); await group("話者A").getByRole("button",{ name: "確定",exact: true }).waitFor({ state: "hidden" });
  check((await payload()).speaker_names.length === 1 && (await payload()).utterances.length === 0 && await name("話者B").inputValue() === "別の未確定名" && await field.inputValue() === "別の未確定本文" && (await writes()).length === 1,"one name confirmation saved unrelated buffers");
  check(await name("話者A").evaluate(e => document.activeElement === e),"name confirmation lost focus when its action button was removed");
  check(await editor.getByRole("combobox",{ name: "話者で絞り込み",exact: true }).locator('option[value="speaker-a"]').textContent() === "入力中の名前","confirmed name did not propagate by speaker ID");
  results.push("one confirmation persists only its name and updates matching labels, retaining other names and text");
  await cancel("話者B"); await field.press("Escape"); await page.waitForTimeout(400);
  check((await writes()).length === 1 && await name("話者B").inputValue() === "佐藤","cancelling unpublished names wrote or changed another name");
  results.push("cancelling an unpublished name restores its saved display without publishing other input");
  await name("話者A").fill(""); await name("話者A").press("Enter"); await waitName("speaker-a",null);
  check(await editor.getByRole("checkbox",{ name: "話者A",exact: true }).first().isVisible(),"cleared name revived base name");
  results.push("Enter confirms an empty display name as null and returns to the original speaker label");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 250; });
  await name("話者A").fill("最初の確定名"); await commit("話者A"); await name("話者A").fill("保存待ち中の新しい名"); await waitName("speaker-a","最初の確定名");
  await group("話者A").getByRole("button",{ name: "確定",exact: true }).waitFor({ state: "visible" }); await page.waitForTimeout(900);
  check(await name("話者A").inputValue() === "保存待ち中の新しい名" && (await writes()).length === 1 && await editor.getByText("未保存",{ exact: true }).isVisible(),"old response replaced or implicitly confirmed newer name");
  await commit("話者A"); await waitName("speaker-a","保存待ち中の新しい名");
  results.push("a delayed acknowledgement preserves newer name input; a second explicit confirmation is required");

  await boot(); await page.evaluate(() => { window.monBrowserFixture.persistenceDelayMs = 250; });
  await name("話者A").fill("別欄へ移動する前の確定名"); await commit("話者A"); await name("話者B").fill("保存待ち中も別欄に入力"); await waitName("speaker-a","別欄へ移動する前の確定名");
  await group("話者A").getByRole("button",{ name: "確定",exact: true }).waitFor({ state: "hidden" });
  check(await name("話者B").evaluate(e => document.activeElement === e) && (await payload()).speaker_names.length === 1,"late name confirmation stole focus or published the other name");
  results.push("an acknowledgement removes confirmed controls without stealing focus from a newer name input");

  await boot(); await name("話者A").fill("保存失敗した名前"); await page.evaluate(() => { window.monBrowserFixture.failSave = true; }); await commit("話者A");
  await editor.getByRole("button",{ name: "保存を再試行",exact: true }).waitFor(); await name("話者B").fill("保存しない別名"); await cancel("話者A");
  await group("話者A").getByRole("button",{ name: "キャンセル",exact: true }).waitFor({ state: "hidden" });
  check((await writes()).length === 1 && await name("話者A").inputValue() === "田中" && await name("話者B").inputValue() === "保存しない別名" && (await payload()).speaker_names.length === 0,"failed-name cancellation retried or discarded unrelated name");
  results.push("failed confirmation cancels only after a read proves unchanged state; other name input remains");

  await boot(); await name("話者A").fill("応答を失う名前"); await page.evaluate(() => { window.monBrowserFixture.lostTranscriptSaveResponse = true; }); await commit("話者A");
  await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).waitFor(); await cancel("話者A");
  await editor.getByRole("alert").filter({ hasText: "先行する保存結果を確認できません" }).waitFor();
  check(await name("話者A").inputValue() === "応答を失う名前" && (await writes()).length === 1,"unknown advanced result allowed blind cancellation");
  await name("話者A").fill("結果確認後に確定する名"); await editor.getByRole("button",{ name: "本文の保存結果を確認",exact: true }).click(); await page.waitForTimeout(900);
  check((await writes()).length === 1 && await name("話者A").inputValue() === "結果確認後に確定する名","read-only result check published newer name");
  await commit("話者A"); await waitName("speaker-a","結果確認後に確定する名");
  results.push("lost response refuses blind cancellation; read-only result checking never publishes new input");

  await boot(); await name("話者A").fill("再試行する確認済み名"); await page.evaluate(() => { window.monBrowserFixture.failSave = true; }); await commit("話者A");
  await editor.getByRole("button",{ name: "保存を再試行",exact: true }).waitFor(); await name("話者A").fill("まだ確定しない最新名"); await name("話者B").fill("未確定の別名");
  await editor.getByRole("button",{ name: "00:00:00の発言を編集",exact: true }).click(); await field.fill("再試行で確定しない本文");
  await page.evaluate(() => { window.monBrowserFixture.failSave = false; }); await editor.getByRole("button",{ name: "保存を再試行",exact: true }).click(); await waitName("speaker-a","再試行する確認済み名"); await page.waitForTimeout(900);
  check((await payload()).speaker_names.length === 1 && (await payload()).utterances.length === 0 && (await writes()).length === 2 && await name("話者A").inputValue() === "まだ確定しない最新名" && await name("話者B").inputValue() === "未確定の別名" && await field.inputValue() === "再試行で確定しない本文","retry implicitly published newer buffers");
  results.push("explicit retry resolves only the frozen confirmed payload, not newer names or open text");

  await boot(); await name("話者A").fill("移動保存する名前"); await name("話者B").fill("移動保存する別名");
  await page.evaluate(async () => { const { flushPendingEdits } = await import("/src/draft-save.ts"); await flushPendingEdits(); });
  await waitName("speaker-a","移動保存する名前"); const version = await page.evaluate(() => window.monBrowserFixture.draft.lockVersion); await name("話者A").press("Escape");
  await page.waitForFunction(n => window.monBrowserFixture.draft.lockVersion === n + 1,version);
  check(await name("話者A").inputValue() === "田中" && (await payload()).speaker_names.length === 1 && (await payload()).speaker_names[0].display_name === "移動保存する別名","barrier cancellation erased another name or old history");
  results.push("save barriers acknowledge all names; later Escape restores only its starting name as a new version");

  await boot(); await name("話者A").fill("移動失敗でも保持する名"); await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
  await page.getByRole("navigation",{ name: "メインナビゲーション",exact: true }).getByRole("button",{ name: "設定",exact: true }).click();
  await page.getByRole("alert").filter({ hasText: "編集内容はこの画面に保持" }).waitFor();
  check(await name("話者A").inputValue() === "移動失敗でも保持する名" && (await payload()).speaker_names.length === 0,"failed navigation lost name or changed screen");
  results.push("failed name flush blocks navigation and retains its input for explicit retry");

  await boot(); await name("話者A").fill("再読込で破棄する名"); await editor.getByRole("button",{ name: "再読込",exact: true }).click();
  const dialog = page.getByRole("dialog",{ name: "文字起こし編集の破棄を確認",exact: true }); await dialog.getByRole("button",{ name: "確認をキャンセル",exact: true }).click();
  check(await name("話者A").inputValue() === "再読込で破棄する名" && (await writes()).length === 0,"reload confirmation cancellation discarded or saved name");
  await editor.getByRole("button",{ name: "再読込",exact: true }).click(); await dialog.getByRole("button",{ name: "入力を破棄して再読込する",exact: true }).click(); await dialog.waitFor({ state: "hidden" });
  check(await name("話者A").inputValue() === "田中" && (await writes()).length === 0,"explicit discard published its unwanted name");
  results.push("reload asks before discarding names; cancellation keeps them and consented discard makes no name write");

  await boot(); await name("話者A").fill("IME中の名前");
  for(const args of [{ key: "Enter",isComposing: true },{ key: "Escape",keyCode: 229 },{ key: "Enter",repeat: true }]) check(await name("話者A").evaluate((e,args) => e.dispatchEvent(new KeyboardEvent("keydown",{ ...args,bubbles: true,cancelable: true })),args),"IME/repeated name key was consumed");
  check((await writes()).length === 0 && await name("話者A").inputValue() === "IME中の名前","IME confirmed or cancelled a name");
  const modifier = await page.evaluate(() => navigator.platform.includes("Mac") ? "Meta" : "Control");
  await name("話者A").press("End"); await page.keyboard.type("X"); await name("話者A").press(`${modifier}+z`);
  check(await name("話者A").inputValue() === "IME中の名前" && await page.evaluate(() => !window.monBrowserFixture.calls.some(c => c.command === "undo_transcript_edit")),"native name Undo invoked persisted history");
  await name("話者A").press("Escape"); check(await name("話者A").inputValue() === "田中","Escape did not restore name baseline");
  results.push("IME and repeats are ignored; native text Undo remains native and Escape cancels the name session");

  await boot(); await name("話者A").fill("幅とテーマを変えても保持する名");
  for(const theme of ["light","dark"]) for(const width of [960,1200]) {
    await page.setViewportSize({ width,height: 900 }); await detailUI.closeSpeakerPanel(); await page.getByRole("banner").getByRole("combobox",{ name: "表示テーマ",exact: true }).selectOption(theme); await detailUI.openSpeakerPanel(); await name("話者A").focus();
    check(await group("話者A").evaluate(e => e.scrollWidth <= e.clientWidth && e.querySelector("input").getBoundingClientRect().right <= e.getBoundingClientRect().right) && await name("話者A").inputValue() === "幅とテーマを変えても保持する名","name layout overflowed or lost input");
    await group("話者A").scrollIntoViewIfNeeded(); await page.waitForTimeout(150); await page.screenshot({ path: `speaker-name-${theme}-${width}.png` });
  }
  results.push("light/dark at 960px and 1200px retain name input with contained labelled controls");
  return { passed: results.length,results };
}
