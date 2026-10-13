// Actual current settings/plain editor with synthetic persistence, no OS changes.
async(page,capturePath)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const settings=()=>page.getByRole("region",{name:"表示設定",exact:true});
  const select=()=>settings().getByRole("combobox",{name:"テーマ",exact:true});
  const waitTheme=theme=>page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
  const count=()=>page.evaluate(()=>window.themeFixture.calls.filter(c=>c.command==="save_ui_preferences").length);
  const openSettings=async()=>{await page.locator('.sidebar').getByRole("button",{name:"設定",exact:true}).click();await select().waitFor();};
  const issue=()=>page.locator('.toast-host').getByRole("alert").filter({hasText:"表示設定の読込・保存"});
  await page.evaluate(()=>window.themeFixture.reset());await page.reload();await openSettings();
  await page.emulateMedia({colorScheme:"light"});await waitTheme("light");
  await page.emulateMedia({colorScheme:"dark"});await waitTheme("dark");check(await count()===0,"OS follow wrote preferences");
  await select().selectOption("light");await waitTheme("light");await page.emulateMedia({colorScheme:"dark"});await waitTheme("light");
  await select().selectOption("dark");await waitTheme("dark");await page.reload();await waitTheme("dark");await openSettings();
  check(await select().inputValue()==="dark","saved theme lost on reload");
  results.push("three choices, OS follow without writes, explicit override and reload persistence");
  await select().selectOption("system");await page.emulateMedia({colorScheme:"light"});await waitTheme("light");
  await page.locator('.sidebar').getByRole("button",{name:"会議一覧",exact:true}).click();await page.locator('.meeting-name').first().click();
  await page.getByRole("tab",{name:"議事録",exact:true}).click();await page.getByRole("button",{name:"議事録を編集",exact:true}).click();
  const text=page.getByRole("textbox",{name:"議事録のMarkdown本文",exact:true});
  await text.fill("# 編集中の点検日\n\n人間の未保存の本文を保持します。\n");
  await text.evaluate(node=>{window.themeFixture.plainNode=node;node.setSelectionRange(2,5);});
  for(const theme of ["light","dark"]){
    await page.emulateMedia({colorScheme:theme});await waitTheme(theme);
    check(await text.evaluate(node=>node===window.themeFixture.plainNode&&node.selectionStart===2&&node.selectionEnd===5),"theme remounted plain input or lost selection");
    check((await text.inputValue()).includes("未保存"),"theme lost pending Markdown");
    check(await page.evaluate(()=>window.themeFixture.calls.filter(c=>c.command==="save_minutes_markdown").length)===0,"theme autosaved human text");
    await page.screenshot({path:capturePath(`plain-theme-${theme}.png`)});
  }
  results.push("plain Markdown node, text and cursor survive OS theme changes without automatic saving");
  await page.getByRole("region",{name:"議事録",exact:true}).getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存済み"}).first().waitFor();
  check(await page.evaluate(()=>window.themeFixture.savedMinutes().markdown.includes("未保存")),"explicit human save missing");
  await openSettings();
  check(await settings().getByRole("combobox").count()===1&&await settings().getByRole("checkbox").count()===0&&await page.getByRole("banner").getByRole("combobox").count()===0,"removed reading controls or header theme remain");
  results.push("theme exists only in settings, without reading-size/follow controls");
  await select().selectOption("dark");await waitTheme("dark");
  const mini=await page.context().newPage();
  try{
    await mini.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/theme-browser-fixture.js");</script>');await route.fulfill({response,body});});
    await mini.goto("http://127.0.0.1:1420/?window=recorder");await mini.waitForFunction(()=>document.documentElement.dataset.theme==="dark");
    check(await mini.evaluate(()=>{const p=window.themeFixture.stored().payload;return p.schemaVersion===1&&Object.keys(p).sort().join(',')==="schemaVersion,theme";}),"mini adopted removed preferences");
    await page.evaluate(()=>window.themeFixture.suppressEvents=true);await select().selectOption("light");await waitTheme("light");await mini.waitForFunction(()=>document.documentElement.dataset.theme==="light");
    check(await mini.evaluate(()=>window.themeFixture.calls.filter(c=>c.command==="save_ui_preferences").length)===0,"mini wrote preferences");
  }finally{await mini.close();}
  results.push("separate synthetic mini panel reads theme-only preferences and follows confirmed changes without writes");
  await page.evaluate(()=>window.themeFixture.mode="fail");await select().selectOption("dark");await issue().waitFor();await waitTheme("light");
  check(await select().inputValue()==="dark","failed selection was discarded");const failed=await count();
  await page.evaluate(()=>window.themeFixture.emitHint({theme:"dark",lockVersion:99999}));await waitTheme("light");check(await count()===failed,"hint replayed failed save");
  await page.evaluate(()=>window.themeFixture.mode="success");await settings().getByRole("button",{name:"テーマの保存を再試行",exact:true}).click();await waitTheme("dark");
  results.push("failed save preserves selection and confirmed colors; hints do not write; explicit retry resolves");
  await page.evaluate(()=>window.themeFixture.mode="lost");const before=await count();await select().selectOption("light");await waitTheme("light");check(await count()===before+1,"lost acknowledgement duplicated save");
  await page.evaluate(()=>window.themeFixture.mode="lostUnreadable");await select().selectOption("dark");await issue().filter({hasText:"保存結果を確認できません"}).waitFor();check(await select().isDisabled(),"unknown commit allows another save");
  const unknown=await count();await page.evaluate(()=>window.themeFixture.readFail=false);
  await waitTheme("dark");check(await count()===unknown,"authoritative polling resaved unknown commit");
  results.push("lost and unknown acknowledgements recover by readback, without duplicate writes");
  await page.evaluate(async()=>{window.themeFixture.mode="success";window.audioDeletionFixture.blocked=true;await window.dataFixture.changed({});});
  await page.waitForFunction(()=>document.querySelector('.theme-choice select')?.disabled);check(await select().isDisabled(),"maintenance write fence ignored");
  results.push("data maintenance disables the sole theme writer while retaining confirmed colors");
  return {passed:results.length,results};
}
