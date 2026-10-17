// Production settings with browser-fixture.js IPC; no user data or native model.
async page => {
  const panel = page.getByRole("region", {name:"議事録生成プロンプト",exact:true});
  const extract = panel.getByRole("textbox", {name:"作成用プロンプト",exact:true});
  const merge = panel.getByRole("textbox", {name:"統合用プロンプト",exact:true});
  const check = (value, message) => { if (!value) throw Error(message); };
  const results = [];
  const saved = () => panel.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  const payload = () => page.evaluate(() => window.monBrowserFixture.promptPreferences.payload);
  await page.reload(); await page.locator(".sidebar").getByRole("button",{name:"設定",exact:true}).click(); await saved();
  check(await extract.inputValue()==="既定の作成用プロンプト", "default extract absent");
  await extract.fill("表形式で自由にまとめる"); await merge.fill("表を統合する"); await saved();
  check((await payload()).extractPrompt==="表形式で自由にまとめる", "extract not saved");
  check((await payload()).mergePrompt==="表を統合する", "merge not saved");
  await panel.getByRole("button", {name:"作成用プロンプトをデフォルトに戻す",exact:true}).click(); await saved();
  check(await extract.inputValue()==="既定の作成用プロンプト" && await merge.inputValue()==="表を統合する", "reset changed another field");
  results.push("independent prompt editing, autosave and reset");

  await page.evaluate(() => window.monBrowserFixture.failPromptSave=true);
  await extract.fill("失敗しても保持する入力");
  await page.getByRole("complementary",{name:"通知",exact:true}).getByRole("alert").filter({hasText:"模擬プロンプト保存失敗"}).waitFor();
  check(await extract.inputValue()==="失敗しても保持する入力", "save failure discarded input");
  await page.evaluate(() => window.monBrowserFixture.failPromptSave=false);
  await panel.getByRole("button",{name:"保存を再試行",exact:true}).click();await saved();
  await page.evaluate(() => window.monBrowserFixture.losePromptResponse=true);
  await extract.fill("応答欠落でも保持する");
  await page.getByRole("complementary",{name:"通知",exact:true}).getByRole("alert").filter({hasText:"模擬プロンプト応答欠落"}).waitFor();
  await extract.fill("応答欠落後の追記");
  await panel.getByRole("button",{name:"保存を再試行",exact:true}).click();await saved();
  check((await payload()).extractPrompt==="応答欠落後の追記", "lost response displaced later text");
  results.push("failed and lost save responses retain later edits and recover");

  const version = await page.evaluate(() => window.monBrowserFixture.promptPreferences.lockVersion);
  await extract.dispatchEvent("compositionstart"); await extract.fill("日本語変換中");
  await page.waitForTimeout(900);
  check(await page.evaluate(() => window.monBrowserFixture.promptPreferences.lockVersion)===version, "saved during IME composition");
  await extract.dispatchEvent("compositionend");await saved();
  check((await payload()).extractPrompt==="日本語変換中", "composition end did not save");
  results.push("IME composition waits before autosave");

  await page.evaluate(() => { const pref=window.monBrowserFixture.promptPreferences;pref.lockVersion++;pref.payload.extractPrompt="別の画面の更新"; });
  await extract.fill("競合しても保持する入力");
  await page.getByRole("complementary",{name:"通知",exact:true}).getByRole("alert").filter({hasText:"競合"}).waitFor();
  await panel.getByRole("button",{name:"保存を再試行",exact:true}).click();
  await page.getByRole("complementary",{name:"通知",exact:true}).getByRole("alert").filter({hasText:"別の内容"}).waitFor();
  check(await extract.inputValue()==="競合しても保持する入力", "conflict discarded draft");
  await panel.getByRole("button",{name:"議事録プロンプト設定を再読込",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"プロンプト入力の破棄を確認",exact:true});
  await dialog.getByRole("button",{name:"入力を破棄して再読込する",exact:true}).click();await saved();
  check(await extract.inputValue()==="別の画面の更新", "reload did not read conflict winner");
  results.push("conflict blocks overwrite and supports explicit reload");

  await extract.fill("画面移動の直前に保存する入力");
  await page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true}).click();
  await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
  check((await payload()).extractPrompt==="画面移動の直前に保存する入力", "navigation did not await save");
  await page.locator(".sidebar").getByRole("button",{name:"設定",exact:true}).click(); await saved();
  check(await extract.inputValue()==="画面移動の直前に保存する入力", "navigation lost saved setting");
  await panel.screenshot({path:"output/playwright/minutes-prompt-settings.png"});
  results.push("navigation flushes pending prompt settings and preserves them on return");
  return results;
}
