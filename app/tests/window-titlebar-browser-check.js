// Called by a Playwright runner against the Vite dev server.
async (page, capturePath = name => name) => {
  const results=[], check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    await route.fulfill({contentType:"text/html",body:'<!doctype html><html lang="ja"><head><meta charset="utf-8"><title>mon — 会議記録</title></head><body><div id="root"></div><script type="module" src="/tests/window-titlebar-browser-fixture.tsx"></script></body></html>'});
  });
  const boot=async(query="")=>{await page.goto(`http://127.0.0.1:1420/?${query}`);};
  const chrome=()=>page.getByRole("group",{name:"ウィンドウ操作",exact:true});
  const callCount=command=>page.evaluate(command=>window.titlebarFixture.calls.filter(c=>c.command===command).length,command);
  await page.setViewportSize({width:1280,height:820}); await boot();
  await chrome().getByText("MON QA — 会議記録",{exact:true}).waitFor();
  const settings=page.getByRole("region",{name:"表示設定",exact:true});
  await settings.waitFor();
  for(const color of ["monochrome","purple"]) for(const mode of ["light","dark"]) {
    await settings.getByRole("combobox",{name:"カラー",exact:true}).selectOption(color);
    await settings.getByRole("combobox",{name:"表示モード",exact:true}).selectOption(mode);
    await page.waitForFunction(({color,mode})=>document.documentElement.dataset.theme===mode&&document.documentElement.dataset.colorTheme===color,{color,mode});
    check(await chrome().evaluate(el=>getComputedStyle(el).backgroundColor===getComputedStyle(document.querySelector(".topbar")).backgroundColor),`${color}/${mode} background mismatch`);
    check(await chrome().evaluate(el=>el.getBoundingClientRect().height===32),"titlebar is not 32 CSS px");
  }
  await settings.getByRole("combobox",{name:"表示モード",exact:true}).selectOption("system");
  for(const mode of ["light","dark"]) {await page.emulateMedia({colorScheme:mode});await page.waitForFunction(mode=>document.documentElement.dataset.theme===mode,mode);check(await chrome().evaluate(el=>getComputedStyle(el).backgroundColor===getComputedStyle(document.querySelector(".topbar")).backgroundColor),"system theme mismatch");}
  results.push("four theme combinations and live system changes exactly match the header");
  await chrome().getByRole("button",{name:"最大化",exact:true}).click();
  await chrome().getByRole("button",{name:"元のサイズに戻す",exact:true}).waitFor();
  await chrome().getByRole("button",{name:"元のサイズに戻す",exact:true}).click();
  await chrome().getByRole("button",{name:"最大化",exact:true}).waitFor();
  await page.evaluate(()=>window.titlebarFixture.resize(true));
  await chrome().getByRole("button",{name:"元のサイズに戻す",exact:true}).waitFor();
  await page.evaluate(()=>window.titlebarFixture.resize(false));
  await chrome().getByRole("button",{name:"最大化",exact:true}).waitFor();
  await chrome().getByRole("button",{name:"最小化",exact:true}).focus(); await page.keyboard.press("Enter");
  check(await callCount("plugin:window|minimize")===1,"keyboard minimize did not use native minimize");
  await chrome().locator(".windows-titlebar-drag").click();
  check(await callCount("plugin:window|start_dragging")===1,"drag region did not start native drag");
  await chrome().locator(".windows-titlebar-drag").dblclick();
  await chrome().getByRole("button",{name:"元のサイズに戻す",exact:true}).waitFor();
  await chrome().getByRole("button",{name:"ウィンドウを閉じる",exact:true}).click();
  check(await callCount("plugin:window|close")===1,"close bypassed native CloseRequested");
  check(await callCount("finish_main_close")===0,"titlebar forced application exit");
  await page.evaluate(()=>{window.titlebarFixture.fail="plugin:window|minimize";});
  await chrome().getByRole("button",{name:"最小化",exact:true}).click();
  await chrome().getByRole("alert").filter({hasText:"合成ウィンドウ操作失敗"}).waitFor();
  check(await chrome().getByRole("button",{name:"ウィンドウを閉じる",exact:true}).isEnabled(),"failed operation disabled controls");
  await page.evaluate(()=>{window.titlebarFixture.fail=null;});
  await chrome().getByRole("button",{name:"最小化",exact:true}).click();
  check(await chrome().getByRole("alert").count()===0,"successful retry retained action error");
  results.push("native actions, double click, external maximize events, keyboard operation and failure recovery");
  for(const size of [{width:1280,height:820},{width:960,height:640}]) {
    await page.setViewportSize(size);
    check(await page.locator(".app-layout").evaluate(el=>{const r=el.getBoundingClientRect();return r.top===32&&r.bottom===innerHeight&&document.documentElement.scrollHeight===innerHeight;}),"content exceeds available viewport");
    check(await page.locator(".meeting-playback-dock").evaluate(el=>el.getBoundingClientRect().bottom<=innerHeight),"player clipped below viewport");
    await page.getByRole("button",{name:"確認を開く",exact:true}).click();
    const modal=page.getByRole("dialog",{name:"タイトルバー確認",exact:true}); await modal.waitFor();
    check(await modal.evaluate(el=>{const r=el.getBoundingClientRect();return r.top>=32&&r.bottom<=innerHeight;}),"modal overlaps titlebar or viewport bottom");
    await page.keyboard.press("Escape");
  }
  await page.evaluate(()=>window.titlebarFixture.showToast());
  await page.getByRole("alert").filter({hasText:"タイトルバーの下に表示する通知"}).waitFor();
  check(await page.locator("[data-sonner-toaster]").evaluate(el=>el.getBoundingClientRect().top>=document.querySelector(".topbar").getBoundingClientRect().bottom),"toast overlaps titlebar/header");
  await page.getByRole("button",{name:"通知一覧",exact:true}).click();
  const popover=page.getByRole("dialog",{name:/通知/});await popover.waitFor();
  check(await popover.evaluate(el=>{const r=el.getBoundingClientRect();return r.top>=32&&r.bottom<=innerHeight;}),"notification popover exceeds content viewport");
  await page.keyboard.press("Escape");
  await page.screenshot({path:capturePath("window-titlebar-dark-purple.png")});
  await page.emulateMedia({forcedColors:"active"});
  check(await chrome().evaluate(el=>{
    const probe=document.createElement("div");probe.style.cssText="background:Canvas;color:CanvasText";document.body.append(probe);
    const expected=getComputedStyle(probe),actual=getComputedStyle(el);
    const matches=actual.backgroundColor===expected.backgroundColor&&actual.color===expected.color;probe.remove();return matches;
  }),"forced colors not honored");
  await page.emulateMedia({forcedColors:"none"});
  results.push("minimum window size, player, scrolling, modal, notifications and forced colors fit correctly");
  for(const phase of ["starting","blocked"]) {
    await boot(`phase=${phase}`); await chrome().waitFor();
    await page.getByRole("main",{name:phase==="starting"?"起動確認":"起動診断",exact:true}).waitFor();
    check(await settings.count()===0,"startup mounted theme/storage provider before admission");
    await chrome().getByRole("button",{name:"ウィンドウを閉じる",exact:true}).click();
    check(await callCount("plugin:window|close")===1,`close unavailable during ${phase}`);
    check(await callCount("get_ui_preferences")===0,"startup titlebar read storage preferences");
  }
  for(const query of ["label=recorder","label=recorder-info","label=recording-confirmation","platform=MacIntel","native=false"]) {
    await boot(query); await page.locator(".app-layout").waitFor();
    check(await chrome().count()===0,`titlebar appeared for ${query}`);
    check(await callCount("plugin:window|title")===0,`native titlebar calls for ${query}`);
    check(await page.locator(".app-layout").evaluate(el=>el.getBoundingClientRect().top===0),"non-Windows content gained titlebar offset");
  }
  results.push("startup loading/errors retain controls; macOS, browser and auxiliary windows remain unchanged");
  return results;
}
