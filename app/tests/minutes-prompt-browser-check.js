// Production prompt screen with browser-fixture.js IPC; no user data or native model.
async page => {
  const panel = page.getByRole("region", {name:"議事録生成プロンプト",exact:true});
  const extract = panel.getByRole("textbox", {name:"作成用プロンプト",exact:true});
  const merge = panel.getByRole("textbox", {name:"統合用プロンプト",exact:true});
  const check = (value, message) => { if (!value) throw Error(message); };
  const results = [];
  const saved = () => panel.getByRole("status").filter({hasText:"保存済み"}).waitFor();
  const payload = () => page.evaluate(() => window.monBrowserFixture.promptPreferences.payload);
  await page.reload(); await page.locator(".sidebar").getByRole("button",{name:"プロンプト",exact:true}).click(); await saved();
  check(await page.getByRole("heading",{name:"プロンプト",exact:true}).isVisible(), "prompt page heading absent");
  check(await page.locator('.sidebar button[aria-current="page"]').getAttribute("aria-label")==="プロンプト", "prompt navigation not selected");
  check(await page.getByRole("navigation",{name:"画面の移動"}).locator('[aria-current="page"]').innerText()==="プロンプト", "prompt breadcrumb absent");
  check(!await page.getByRole("region",{name:"保存済み会議",exact:true}).isVisible(), "meeting list leaked into prompts");
  await page.locator(".sidebar").getByRole("button",{name:"設定",exact:true}).click();
  check(!await panel.isVisible(), "prompt editor still visible in settings");
  await page.getByRole("button",{name:"前の画面へ戻る",exact:true}).click(); await saved();
  results.push("standalone prompt navigation, breadcrumb and back; editor absent from settings");
  check(await extract.inputValue()==="既定の作成用プロンプト", "default extract absent");
  await extract.fill("表形式で自由にまとめる"); await merge.fill("表を統合する"); await saved();
  check((await payload()).extractPrompt==="表形式で自由にまとめる", "extract not saved");
  check((await payload()).mergePrompt==="表を統合する", "merge not saved");
  await panel.getByRole("button", {name:"作成用プロンプトをデフォルトに戻す",exact:true}).click(); await saved();
  check(await extract.inputValue()==="既定の作成用プロンプト" && await merge.inputValue()==="表を統合する", "reset changed another field");
  results.push("independent prompt editing, autosave and reset");

  await extract.fill(" ");
  const fieldError = panel.getByText("作成用プロンプトを入力してください。", { exact: true });
  await fieldError.waitFor();
  check(await extract.getAttribute("aria-invalid")==="true" && await merge.getAttribute("aria-invalid")==="false", "prompt error marked the wrong field");
  check(await page.evaluate(async()=>{
    const {toasts}=await import('/src/toast.tsx');
    return !toasts.getHistorySnapshot().some(notice=>notice.message.startsWith("議事録プロンプト設定:"));
  }), "editing added a prompt error to notification history");
  await extract.fill("修正した作成用プロンプト"); await saved();
  check((await payload()).extractPrompt==="修正した作成用プロンプト", "valid correction was not saved");
  await fieldError.waitFor({state:"hidden"});
  results.push("prompt input errors are scoped to the field and a valid correction is saved");

  await page.evaluate(() => window.monBrowserFixture.failPromptSave=true);
  await extract.fill("失敗しても保持する入力");
  await page.getByRole("complementary",{name:"通知",exact:true}).getByRole("alert").filter({hasText:"模擬プロンプト保存失敗"}).waitFor();
  check(await extract.inputValue()==="失敗しても保持する入力", "save failure discarded input");
  await page.locator(".sidebar").getByRole("button",{name:"設定",exact:true}).click();
  check(await panel.isVisible() && await extract.inputValue()==="失敗しても保持する入力", "failed save allowed navigation or discarded input");
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
  await page.locator(".sidebar").getByRole("button",{name:"プロンプト",exact:true}).click(); await saved();
  check(await extract.inputValue()==="画面移動の直前に保存する入力", "navigation lost saved setting");

  results.push("navigation flushes pending prompt settings and preserves them on return");
  await page.locator(".sidebar").getByRole("button",{name:"会議一覧",exact:true}).click();
  await page.locator(".meeting-name").first().click();
  await page.getByRole("tab",{name:"文字起こし",exact:true}).waitFor();
  const meetingTitle = await page.locator(".meeting-detail-status h1").innerText();
  await page.locator(".sidebar").getByRole("button",{name:"プロンプト",exact:true}).click(); await saved();
  check(await page.getByRole("tab",{name:"文字起こし",exact:true}).count()===0, "meeting details leaked into prompts");
  check(!await page.getByRole("region",{name:"会議の共通音声プレーヤー",exact:true}).isVisible(), "meeting audio leaked into prompts");
  check(await page.locator(".meeting-name").count()===0, "meeting list rendered behind prompt editor");
  await page.getByRole("button",{name:"前の画面へ戻る",exact:true}).click();
  await page.getByRole("tab",{name:"文字起こし",exact:true}).waitFor();
  check(await page.locator(".meeting-detail-status h1").innerText()===meetingTitle, "back lost selected meeting");
  results.push("meeting detail to prompts hides meeting content and player; back restores selected meeting");
  await page.locator(".sidebar").getByRole("button",{name:"プロンプト",exact:true}).click(); await saved();
  await page.screenshot({path:"output/playwright/minutes-prompt-screen.png",fullPage:true,animations:"disabled"});
  await page.setViewportSize({width:360,height:740});
  check(await extract.isVisible() && await merge.isVisible(), "prompt editor unavailable on narrow screen");
  check(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth), "prompt screen overflows narrow viewport");
  results.push("narrow prompt screen remains usable without horizontal overflow");
  return results;
}
