// Actual current settings/plain editor with synthetic persistence, no OS changes.
async(page,capturePath)=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  const settings=()=>page.getByRole("region",{name:"表示設定",exact:true});
  const select=()=>settings().getByRole("combobox",{name:"表示モード",exact:true});
  const color=()=>settings().getByRole("combobox",{name:"カラー",exact:true});
  const waitTheme=theme=>page.waitForFunction(theme=>document.documentElement.dataset.theme===theme,theme);
  const count=()=>page.evaluate(()=>window.themeFixture.calls.filter(c=>c.command==="save_ui_preferences").length);
  const openSettings=async()=>{await page.locator('.sidebar').getByRole("button",{name:"設定",exact:true}).click();await select().waitFor();};
  const issue=()=>page.locator('.toast-host').getByRole("alert").filter({hasText:"表示設定の読込・保存"});
  await page.evaluate(()=>window.themeFixture.reset());await page.reload();await openSettings();
  check(await color().inputValue()==="monochrome","initial color is not monochrome");
  await page.emulateMedia({colorScheme:"light"});await waitTheme("light");
  await page.emulateMedia({colorScheme:"dark"});await waitTheme("dark");check(await count()===0,"OS follow wrote preferences");
  await select().selectOption("light");await waitTheme("light");await page.emulateMedia({colorScheme:"dark"});await waitTheme("light");
  await select().selectOption("dark");await waitTheme("dark");await page.reload();await waitTheme("dark");await openSettings();
  check(await select().inputValue()==="dark","saved theme lost on reload");
  results.push("three choices, OS follow without writes, explicit override and reload persistence");
  const palettes = {};
  for (const palette of ["monochrome","purple"]) {
    await color().selectOption(palette);
    await page.waitForFunction(palette=>document.documentElement.dataset.colorTheme===palette,palette);
    for (const mode of ["light","dark"]) {
      await select().selectOption(mode); await waitTheme(mode);
      palettes[palette+"-"+mode]=await page.evaluate(()=>{
        const s=getComputedStyle(document.documentElement), rgb=name=>s.getPropertyValue(name).trim();
        return Object.fromEntries(["--primary","--text","--surface","--focus","--selection","--recording","--warning-text","--danger-text","--success-text"].map(name=>[name,rgb(name)]));
      });
      const tokens=palettes[palette+"-"+mode];
      const luminance=hex=>{let h=hex.slice(1);if(h.length===3)h=[...h].map(x=>x+x).join("");return [0,2,4].map(i=>parseInt(h.slice(i,i+2),16)/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((n,v,i)=>n+v*[.2126,.7152,.0722][i],0);};
      const contrast=(a,b)=>{const x=luminance(a),y=luminance(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);};
      check(contrast(tokens["--text"],tokens["--surface"])>=4.5,"body text contrast too low");
      check(contrast(tokens["--focus"],tokens["--surface"])>=3,"focus contrast too low");
      check(contrast(tokens["--primary"],mode==="light"?"#fff":"#18181b")>=4.5,"primary contrast too low");
      while(await page.locator('.toast-host').getByRole('button',{name:/通知を閉じる$/}).count()) {
        try { await page.locator('.toast-host').getByRole('button',{name:/通知を閉じる$/}).first().press('Enter',{timeout:1000}); }
        catch(cause) { if(await page.locator('.toast-host').getByRole('button',{name:/通知を閉じる$/}).count()) throw cause; }
      }
      await page.locator('.toast-host .toast-card').waitFor({state:"hidden"});
      await page.locator('.content').evaluate(e=>{e.scrollTop=0;});
      await page.screenshot({path:capturePath(`settings-${palette}-${mode}.png`)});
    }
    await page.reload(); await openSettings();
    check(await color().inputValue()===palette&&await select().inputValue()==="dark","color/brightness not persisted independently");
  }
  for(const mode of ["light","dark"]){
    check(palettes["purple-"+mode]["--primary"]===(mode==="light"?"#4f46e5":"#818cf8"),"original purple changed");
    for(const token of ["--recording","--warning-text","--danger-text","--success-text"])check(palettes["purple-"+mode][token]===palettes["monochrome-"+mode][token],"semantic color changed with palette");
  }
  results.push("four palette/brightness combinations retain purple and semantic colors, readable contrast and persistence");
  await color().selectOption("monochrome");
  await page.waitForFunction(()=>document.documentElement.dataset.colorTheme==="monochrome");
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
  for(const palette of ["purple","monochrome"]){
    await page.evaluate(async palette=>{const f=window.themeFixture,s=f.stored();await f.setStored({...s,lockVersion:s.lockVersion+1,payload:{...s.payload,colorTheme:palette}});},palette);
    await page.waitForFunction(palette=>document.documentElement.dataset.colorTheme===palette,palette);
    check(await text.evaluate(node=>node===window.themeFixture.plainNode&&node.selectionStart===2&&node.selectionEnd===5)&& (await text.inputValue()).includes("未保存"),"color change replaced the editor or its selection");
  }
  results.push("cross-window color changes preserve unsaved Markdown and cursor without remounting");
  await page.getByRole("region",{name:"議事録",exact:true}).getByRole("button",{name:"保存",exact:true}).click();
  await page.getByRole("status").filter({hasText:"保存済み"}).first().waitFor();
  check(await page.evaluate(()=>window.themeFixture.savedMinutes().markdown.includes("未保存")),"explicit human save missing");
  await openSettings();
  check(await settings().getByRole("combobox").count()===2&&await settings().getByRole("checkbox").count()===0&&await page.getByRole("banner").getByRole("combobox").count()===0,"removed reading controls or header theme remain");
  results.push("theme exists only in settings, without reading-size/follow controls");
  await select().selectOption("dark");await waitTheme("dark");
  const mini=await page.context().newPage();
  try{
    await mini.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/theme-browser-fixture.js");</script>');await route.fulfill({response,body});});
    await mini.goto("http://127.0.0.1:1420/?window=recorder");await mini.waitForFunction(()=>document.documentElement.dataset.theme==="dark");
    check(await mini.evaluate(()=>{const p=window.themeFixture.stored().payload;return p.schemaVersion===1&&Object.keys(p).sort().join(',')==="colorTheme,schemaVersion,theme"&&document.documentElement.dataset.colorTheme==="monochrome";}),"mini adopted wrong color/preferences");
    await color().selectOption("purple");await mini.waitForFunction(()=>document.documentElement.dataset.colorTheme==="purple");
    await page.evaluate(()=>window.themeFixture.suppressEvents=true);await select().selectOption("light");await waitTheme("light");await mini.waitForFunction(()=>document.documentElement.dataset.theme==="light");
    check(await mini.evaluate(()=>window.themeFixture.calls.filter(c=>c.command==="save_ui_preferences").length)===0,"mini wrote preferences");
  }finally{await mini.close();}
  results.push("separate synthetic mini panel reads both settings and follows confirmed changes without writes");
  await page.evaluate(()=>window.themeFixture.mode="fail");await select().selectOption("dark");await issue().waitFor();await waitTheme("light");
  check(await select().inputValue()==="dark","failed selection was discarded");const failed=await count();
  await page.evaluate(()=>window.themeFixture.emitHint({theme:"dark",lockVersion:99999}));await waitTheme("light");check(await count()===failed,"hint replayed failed save");
  await page.evaluate(()=>window.themeFixture.mode="success");await settings().getByRole("button",{name:"テーマの保存を再試行",exact:true}).click();await waitTheme("dark");
  results.push("failed save preserves selection and confirmed colors; hints do not write; explicit retry resolves");
  await page.evaluate(()=>window.themeFixture.mode="lost");const before=await count();await select().selectOption("light");await waitTheme("light");check(await count()===before+1,"lost acknowledgement duplicated save");
  await page.evaluate(()=>window.themeFixture.mode="lostUnreadable");await select().selectOption("dark");
  const confirmResult = page.locator(".toast-host").getByRole("button", {name:"表示設定の結果を確認", exact:true});
  await confirmResult.waitFor();
  check(await confirmResult.isEnabled(), "unknown commit offers no way to check the result");
  check(await select().isDisabled() && await color().isDisabled(), "unknown commit allows another save");
  const unknown=await count();await page.evaluate(()=>window.themeFixture.readFail=false);
  await waitTheme("dark");check(await count()===unknown,"authoritative polling resaved unknown commit");
  results.push("lost and unknown acknowledgements recover by readback, without duplicate writes");
  await page.evaluate(async()=>{window.themeFixture.mode="success";window.audioDeletionFixture.blocked=true;await window.dataFixture.changed({});});
  await page.waitForFunction(()=>document.querySelector('.theme-choice select')?.disabled);check(await select().isDisabled(),"maintenance write fence ignored");
  results.push("data maintenance disables the sole theme writer while retaining confirmed colors");
  return {passed:results.length,results};
}
