// Independent synthetic log scenarios; no native processes or user storage.
async (page, _capturePath, detailUI) => {
  const results = [], check = (condition, message) => { if (!condition) throw Error(message); };
  const logs = page.locator('section.internal-logs[aria-label="内部ログ"]');
  const list = logs.locator('.internal-logs-scroll[aria-label="ログ一覧"]');
  const toolbar = logs.locator(".internal-logs-toolbar");
  const process = logs.getByRole("tab", {name:"処理経過",exact:true});
  const engine = logs.getByRole("tab", {name:"エンジンログ",exact:true});
  const details = page.getByRole("dialog", {name:"ログ詳細",exact:true});
  const filterToggle = logs.getByRole("button", {name:"ログを絞り込み",exact:true});
  const filterPanel = logs.getByRole("region", {name:"ログの絞り込み",exact:true});
  // Read row geometry/counts directly so large histories are not repeatedly
  // traversed by the accessibility selector engine just to inspect their size.
  const rowState = () => page.evaluate(() => {
    const rows=document.querySelectorAll(".internal-logs tbody tr");
    return {count:rows.length,first:rows[0]?.dataset.logId};
  });
  const waitRows = count => page.waitForFunction(count => document.querySelectorAll(".internal-logs tbody tr").length === count, count);
  const top = () => page.evaluate(() => {
    const element=document.querySelector(".internal-logs-scroll");
    if (!element) throw Error("Log list is not mounted");
    // Enter the top from a scrollable position instead of depending on a
    // previous layout's native scroll event to arm the next history request.
    element.scrollTop=Math.max(9,element.scrollTop); element.dispatchEvent(new Event("scroll"));
    element.scrollTop=0; element.dispatchEvent(new Event("scroll"));
  });
  const idle = () => page.waitForFunction(() => {
    const refresh=document.querySelector('.internal-logs-toolbar button[aria-label="更新"]');
    return !!refresh && !refresh.disabled;
  });
  const reset = async () => {
    await page.goto("http://127.0.0.1:1420/tests/internal-logs-browser.html");
    await page.waitForFunction(() => !!window.internalLogsFixture);
  };
  const open = async () => { await page.getByRole("button", {name:"内部ログ",exact:true}).click(); await idle(); };
  const setTheme = async theme => {
    await page.evaluate(async theme => { const f=window.monBrowserFixture; f.uiPreferences={id:"ui_preferences",payload:{schemaVersion:1,theme,colorTheme:"monochrome"},lockVersion:(f.uiPreferences?.lockVersion??0)+1}; await f.emitUiPreferences(); },theme);
    await page.waitForFunction(theme => document.documentElement.dataset.theme === theme,theme);
  };
  const controls = async () => {
    await detailUI.prepareClipboard();
    await reset(); await open();
    check(await page.getByRole("region", {name:"内部ログ",exact:true}).count() === 1,"log viewer lacks its named region");
    check(await logs.getByRole("tablist", { name:"ログの種類" }).count() === 1, "log kind is not a tablist");
    check(await process.getAttribute("aria-selected") === "true" && await engine.getAttribute("tabindex") === "-1", "tab selection semantics missing");
    check(await process.getAttribute("aria-controls") === await logs.getByRole("tabpanel").getAttribute("id"), "tab panel is not associated");
    await process.focus(); await page.keyboard.press("ArrowRight");
    check(await engine.evaluate(el => el === document.activeElement) && await process.getAttribute("aria-selected") === "true", "arrow did not move tab focus without activation");
    await page.keyboard.press("Enter"); await logs.getByText(/using device MTL0/).waitFor();
    await page.keyboard.press("Home"); await page.keyboard.press("Space");
    await waitRows(200);
    results.push("underlined tabs expose selection and a labelled panel, with keyboard activation");
    for (const name of ["自動更新を一時停止","更新","表示中ログをコピー","最新ログへ移動"]) {
      const button = toolbar.getByRole("button", {name,exact:true});
      check(await button.innerText() === "" && await button.locator("svg").count() === 1 && await button.getAttribute("title") === name, "toolbar icon is missing its accessible name or tooltip");
    }
    check(await toolbar.evaluate(el => { const centers = [...el.children].map(child => { const r=child.getBoundingClientRect(); return r.y+r.height/2; }); return Math.max(...centers)-Math.min(...centers)<2; }), "toolbar is not one row at normal width");
    check(await toolbar.locator("button").count() === 4 && await toolbar.locator("button").nth(3).getAttribute("aria-label") === "最新ログへ移動", "toolbar button order changed");
    check(await logs.getByRole("button", {name:"古いログを読み込む",exact:true}).count() === 0, "manual history button remains");
    await setTheme("light"); await page.screenshot({path:"output/playwright/internal-logs-light.png"});
    await setTheme("dark"); await page.screenshot({path:"output/playwright/internal-logs-dark.png"});
    await setTheme("light");
    results.push("pause, refresh and copy are named icons on one toolbar row, in light and dark themes");
    check(await filterToggle.getAttribute("aria-expanded") === "false" && !await filterPanel.isVisible(), "filters must start collapsed");
    await filterToggle.focus(); await page.keyboard.press("Enter");
    await filterPanel.waitFor();
    check(await filterToggle.getAttribute("aria-controls") === await filterPanel.getAttribute("id"), "filter control is not associated with panel");
    await logs.getByRole("tab", { name: "エンジンログ", exact: true }).click();
    await logs.getByText(/using device MTL0/).first().waitFor();
    check(await page.evaluate(() => window.logMarkupExecuted) === undefined, "raw HTML executed");
    const detailTrigger = logs.getByRole("button", {name:/のログ詳細/});
    await detailTrigger.hover();
    await details.getByText("job-synthetic", {exact:true}).waitFor();
    check(await details.getAttribute("aria-modal") === "false" && await page.locator("dialog[open]").count() === 0, "details opened a blocking modal");
    await details.hover();
    await page.clock.runFor(250);
    check(await details.isVisible(), "moving into details closed the hover popup");
    check(await details.locator("pre").innerText().then(text => text.includes("会議本文🦀")), "details lost complete raw text");
    check(await details.evaluate(el => { const r=el.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight; }), "details overflow the viewport");
    await page.screenshot({path:"output/playwright/internal-logs-detail.png"});
    await page.keyboard.press("Escape"); await details.waitFor({state:"hidden"});
    check(await detailTrigger.evaluate(el => el===document.activeElement), "closing details lost trigger focus");
    await toolbar.getByRole("button", {name:"更新",exact:true}).focus();
    await detailTrigger.focus();
    await details.waitFor();
    check(await detailTrigger.evaluate(el => el === document.activeElement), "focus popup stole keyboard focus");
    await details.getByRole("button", {name:"ログ詳細の説明を閉じる",exact:true}).click();
    await details.waitFor({state:"hidden"});
    await detailTrigger.hover();
    await details.waitFor();
    const bounds=await details.boundingBox();
    await page.mouse.click(bounds.x-8,bounds.y+20); await details.waitFor({state:"hidden"});
    results.push("details open on hover and keyboard focus without a modal, preserve raw text and close with Escape or outside interaction");
    await logs.getByRole("button", { name: "表示中ログをコピー", exact: true }).click();
    await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("status").filter({hasText:"1件のログをコピーしました。"}).waitFor();
    check((await page.evaluate(() => navigator.clipboard.readText())).includes("会議本文🦀"), "copy lost raw output");
    await logs.getByRole("searchbox", { name: "文字列", exact: true }).fill("does-not-exist");
    await logs.getByText(/条件に一致するログはありません/).waitFor();
    await filterToggle.click();
    check(!await filterPanel.isVisible() && (await rowState()).count === 0, "collapsing filters cleared the active filter");
    await filterToggle.click();
    check(await logs.getByRole("searchbox", { name: "文字列", exact: true }).inputValue() === "does-not-exist", "reopening filters lost the input");
    await page.setViewportSize({width:390,height:900});
    check(await filterPanel.evaluate(el => { const r=el.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth; }), "expanded filters overflow narrow screen");
    check(await toolbar.locator("button").evaluateAll(buttons => buttons.every(el => {const r=el.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth;})), "toolbar controls overflow narrow screen");
    await page.screenshot({path:"output/playwright/internal-logs-narrow.png"});
    await page.setViewportSize({width:1200,height:900});
    await logs.getByRole("searchbox", { name: "文字列", exact: true }).fill("");
    await logs.getByText(/using device MTL0/).first().waitFor();
    await logs.getByRole("tab", { name: "処理経過", exact: true }).click();
    await waitRows(200);
    await logs.getByRole("combobox", { name: "重要度", exact: true }).selectOption("warn");
    await waitRows(110);
    await logs.getByRole("searchbox", { name: "会議／ジョブ／操作ID", exact: true }).fill("missing-job");
    await logs.getByText(/条件に一致するログはありません/).waitFor();
    await logs.getByRole("searchbox", { name: "会議／ジョブ／操作ID", exact: true }).fill("");
    await waitRows(110);
    await logs.getByRole("combobox", { name: "処理種別", exact: true }).selectOption("recording");
    await logs.getByText(/条件に一致するログはありません/).waitFor();
    await logs.getByRole("combobox", { name: "処理種別", exact: true }).selectOption("");
    await waitRows(110);
    await logs.getByLabel("開始日時", { exact: true }).fill("2026-10-17T00:00");
    await logs.getByLabel("終了日時", { exact: true }).fill("2026-10-16T00:00");
    await logs.getByRole("alert").filter({ hasText: "終了日時は開始日時以降" }).waitFor();
    await logs.getByLabel("開始日時", { exact: true }).fill("");
    await logs.getByLabel("終了日時", { exact: true }).fill("");
    await waitRows(110);
    await page.evaluate(() => { window.internalLogsFixture.hold = true; });
    await logs.getByRole("button", { name: "更新", exact: true }).click();
    await page.waitForFunction(() => window.internalLogsFixture.pending.length === 1);
    await logs.getByRole("searchbox", { name: "文字列", exact: true }).fill("工程 1091");
    await page.waitForFunction(() => window.internalLogsFixture.pending.length === 2);
    await page.evaluate(() => window.internalLogsFixture.release());
    await waitRows(1);
    await logs.getByText("ログの工程 1091", { exact: true }).waitFor();
    await logs.getByRole("searchbox", { name: "文字列", exact: true }).fill("");
    await waitRows(110);
    await page.evaluate(() => { window.internalLogsFixture.fail = true; });
    await logs.getByRole("button", { name: "更新", exact: true }).click();
    await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("alert").filter({ hasText: "合成ログ取得エラー" }).waitFor();
    check((await rowState()).count === 110, "fetch failure cleared displayed logs");
    await page.evaluate(() => { window.internalLogsFixture.fail = false; window.internalLogsFixture.status = { storageError: "合成の保存失敗", droppedEntries: 7, historyTruncated: true }; });
    await logs.getByRole("button", { name: "更新", exact: true }).click();
    await logs.getByRole("status").filter({ hasText: "合成の保存失敗" }).waitFor();
    await logs.getByText(/7件のログを記録できませんでした/).waitFor();
    await logs.getByText(/保持期間または容量上限/).waitFor();
    await page.getByRole("button", { name: "前の画面へ戻る", exact: true }).click();
    await page.getByRole("heading", { name: "会議一覧", exact: true }).waitFor();
    await page.getByRole("button", { name: "会議A", exact: true }).click();
    const editor = page.getByRole("region", { name: "文字起こし編集", exact: true });
    const input = editor.getByRole("textbox", { name: "00:00:00の発言", exact: true });
    await editor.getByRole("button", { name: "00:00:00の発言を編集", exact: true }).click();
    await input.fill("内部ログへ移動する前の未保存編集");
    await page.evaluate(() => { window.monBrowserFixture.failSave = true; });
    await page.getByRole("button", { name: "内部ログ", exact: true }).click();
    await editor.getByRole("button", { name: "保存を再試行", exact: true }).waitFor();
    check(await input.inputValue() === "内部ログへ移動する前の未保存編集" && await logs.count() === 0, "log navigation lost unsaved edits");
    await page.evaluate(() => { window.monBrowserFixture.failSave = false; });
    await editor.getByRole("button", { name: "保存を再試行", exact: true }).click();
    await input.waitFor({ state: "hidden" });
    await page.getByRole("button", { name: "内部ログ", exact: true }).click();
    await logs.getByRole("heading", { name: "内部ログ", exact: true }).waitFor();
    results.push("filter disclosure, keyboard, retained values and narrow layout", "sidebar navigation", "raw details, copy and inert markup", "text, ID and level filtering", "fetch failure retains rows", "storage loss notices", "stale filter replies", "date/category filters", "unsaved edit protection");
  };

  const history = async () => {
    await reset(); await open();
    await waitRows(200);
    await logs.getByRole("button", { name: "自動更新を一時停止", exact: true }).click();
    await page.evaluate(() => window.internalLogsFixture.add({ message: "一時停止中の追記" }));
    await page.clock.runFor(1200);
    check(await logs.getByText("一時停止中の追記", { exact: true }).count() === 0, "paused viewer updated");
    await logs.getByRole("button", { name: "自動更新を再開", exact: true }).click();
    await logs.getByText("一時停止中の追記", { exact: true }).waitFor();
    await page.evaluate(() => { window.internalLogsFixture.holdOlder = true; });
    await top();
    await page.waitForFunction(() => window.internalLogsFixture.pending.length === 1);
    await logs.getByText("古いログを読み込んでいます…", {exact:true}).waitFor();
    const anchor = await list.evaluate(el => {
      const r=el.getBoundingClientRect(), header=el.querySelector("thead").getBoundingClientRect().height;
      const row=[...el.querySelectorAll("tr[data-log-id]")].find(row=>row.getBoundingClientRect().bottom>r.top+header);
      return {id:row.dataset.logId,offset:row.getBoundingClientRect().top-r.top};
    });
    const heldCalls = await page.evaluate(() => window.internalLogsFixture.calls.length);
    await top(); await top();
    check(await page.evaluate(() => window.internalLogsFixture.calls.length) === heldCalls, "history requests overlap");
    await page.evaluate(() => window.internalLogsFixture.release());
    await waitRows(400);
    check(await list.evaluate((el,anchor) => Math.abs(el.querySelector(`tr[data-log-id="${anchor.id}"]`).getBoundingClientRect().top-el.getBoundingClientRect().top-anchor.offset)<2,anchor), "prepending history moved the visible row");
    check(await toolbar.getByRole("button", {name:"自動更新を再開",exact:true}).count() === 1, "history did not pause updates");
    await page.evaluate(() => window.internalLogsFixture.add({message:"読んでいる途中の追記"}));
    await page.clock.runFor(1200);
    check(await logs.getByText("読んでいる途中の追記", {exact:true}).count() === 0, "history reading admitted live updates");
    results.push("top scrolling loads older logs once, pauses live updates and preserves the visible row");
    results.push("pause/resume and reading position");
  };

  const pagination = async () => {
    await reset(); await open(); await waitRows(200);
    for (let i=0;i<5;i++) {
      const {first:previous}=await rowState();
      if (previous === "1") break;
      await top();
      await page.waitForFunction(previous => document.querySelector(".internal-logs tbody tr")?.dataset.logId!==previous,previous);
      await idle();
    }
    const oldest=await rowState();
    check(oldest.count === 1000 && oldest.first === "1", "history cap or oldest cursor failed");
    await logs.locator(".internal-logs-notice").filter({hasText:/画面の上限1,000件/}).waitFor();
    await logs.locator(".internal-logs-history-status").filter({hasText:/^これより古いログはありません。$/}).waitFor();
    const endCalls=await page.evaluate(() => window.internalLogsFixture.calls.length);
    await top(); await page.clock.runFor(1200);
    check(await page.evaluate(() => window.internalLogsFixture.calls.length) === endCalls, "exhausted history kept requesting");
    await page.evaluate(() => window.internalLogsFixture.add({message:"読んでいる途中の追記"}));
    await toolbar.getByRole("button", {name:"最新ログへ移動",exact:true}).click();
    await logs.getByText("読んでいる途中の追記", {exact:true}).waitFor();
    check((await rowState()).count === 200 && await toolbar.getByRole("button",{name:"自動更新を一時停止",exact:true}).count() === 1, "latest did not resume newest view");
    results.push("oldest boundary stops paging, the 1000-row cap holds, and latest resumes live following");
    results.push("initial page and pagination cap");
  };

  const live = async () => {
    await reset();
    await page.evaluate(() => { window.internalLogsFixture.entries = window.internalLogsFixture.entries.slice(0, 50); });
    await page.getByRole("button", { name: "内部ログ", exact: true }).click();
    await waitRows(50);
    await logs.getByRole("button", { name: "自動更新を一時停止", exact: true }).click();
    await page.evaluate(() => { for (let i = 0; i < 1100; i++) window.internalLogsFixture.add(); });
    for (let i = 0; i < 6; i++) {
      await logs.getByRole("button", { name: "更新", exact: true }).click();
      await logs.getByText(`ログの工程 ${Math.min(50 + (i + 1) * 200, 1150)}`, { exact: true }).waitFor();
    }
    check((await rowState()).count === 1000, "live delta cap failed");
    await top();
    await logs.getByText("ログの工程 1", { exact: true }).waitFor();
    check((await rowState()).count === 1000, "trimmed history could not be recovered");
    results.push("history recovery after live row trimming");
  };

  const recovery = async () => {
    await reset(); await open();
    await page.evaluate(() => { window.internalLogsFixture.failOlder = true; });
    await top();
    await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("alert").filter({hasText:"合成の履歴取得エラー"}).waitFor();
    const failedCalls=await page.evaluate(() => window.internalLogsFixture.calls.length);
    await top(); await page.clock.runFor(1200);
    check(await page.evaluate(() => window.internalLogsFixture.calls.length) === failedCalls && (await rowState()).count === 200, "failed history retried itself or cleared rows");
    await page.evaluate(() => { window.internalLogsFixture.failOlder = false; });
    await toolbar.getByRole("button",{name:"更新",exact:true}).click();
    await waitRows(400);
    results.push("history errors retain rows, stop automatic retries and retry the same cursor with refresh");
    await page.evaluate(() => { window.internalLogsFixture.stallOlder = true; });
    await top(); await page.getByRole("complementary", {name:"通知",exact:true}).getByRole("alert").filter({hasText:"読み込み位置が進みません"}).waitFor();
    const stalledCalls=await page.evaluate(() => window.internalLogsFixture.calls.length);
    await page.clock.runFor(1200);
    check(await page.evaluate(() => window.internalLogsFixture.calls.length) === stalledCalls, "nonadvancing cursor looped");
    results.push("a nonadvancing history cursor stops rather than looping");
    await reset();
    await page.evaluate(() => {
      const f=window.internalLogsFixture,status=f.status;
      f.pages=[
        {entries:[],olderCursor:900,newerCursor:1101,hasMore:true,expired:false,status},
        {entries:[],olderCursor:700,newerCursor:1101,hasMore:true,expired:false,status},
        {entries:[{...f.entries[499],message:"空ページの先の一致ログ"}],olderCursor:500,newerCursor:1101,hasMore:false,expired:false,status},
      ];
    });
    await open(); await logs.getByText("空ページの先の一致ログ", {exact:true}).waitFor();
    check(await page.evaluate(() => {const c=window.internalLogsFixture.calls;return c.length===3&&c[1].query.before===900&&c[2].query.before===700;}), "empty scan pages did not advance by cursor");
    results.push("empty native scan pages continue by cursor until a match or the history boundary");
    // An in-flight automatic poll must not swallow a simultaneous top request.
    await reset(); await open();
    await page.evaluate(() => { window.internalLogsFixture.hold=true; });
    await page.waitForFunction(() => window.internalLogsFixture.pending.length===1);
    await top();
    await page.evaluate(() => window.internalLogsFixture.release());
    await page.waitForFunction(() => document.querySelectorAll(".internal-logs tbody tr").length===400);
    results.push("history loading queues behind a live poll and discards its paused acknowledgement");
    // Deliver a top scroll after a live reply is consumed but before React
    // commits its following layout. The user's history request must win.
    await reset(); await open(); await waitRows(200);
    await page.evaluate(() => {
      const invoke=window.__TAURI_INTERNALS__.invoke;
      let injected=false;
      window.__TAURI_INTERNALS__.invoke=async (command,args,options) => {
        const reply=await invoke(command,args,options);
        if(command!=="get_internal_logs" || args.query.after===undefined || injected) return reply;
        const entries=reply.entries;
        Object.defineProperty(reply,"entries",{get() {
          if(!injected) {
            injected=true;
            queueMicrotask(() => {
              const element=document.querySelector(".internal-logs-scroll");
              element.scrollTop=Math.max(9,element.scrollTop); element.dispatchEvent(new Event("scroll"));
              element.scrollTop=0; element.dispatchEvent(new Event("scroll"));
            });
          }
          return entries;
        }});
        return reply;
      };
    });
    await waitRows(400);
    check((await rowState()).first === "701" && await toolbar.getByRole("button",{name:"自動更新を再開",exact:true}).count()===1, "a completed live reply swallowed the history request");
    results.push("a completed live reply cannot overwrite a top scroll before its layout commits");
    await popup();
  };

  const popup = async () => {
    // An anchored popup closes when its row leaves the visible history cap.
    await reset();
    await page.evaluate(() => { const f=window.internalLogsFixture; f.entries=f.entries.filter(entry => entry.kind === "engine"); });
    await open(); await engine.click();
    await logs.getByText(/using device MTL0/).first().waitFor();
    await engine.focus();
    await logs.getByRole("button", {name:/のログ詳細/}).hover();
    await details.getByText(/会議本文🦀/).waitFor();
    await page.evaluate(() => {for(let i=0;i<1100;i++)window.internalLogsFixture.add({kind:"engine",level:"raw",message:`新着エンジンログ ${i}`});});
    await page.waitForFunction(() => document.querySelectorAll(".internal-logs tbody tr").length===1000 && !document.querySelector('.internal-logs tr[data-log-id="1101"]'));
    await details.waitFor({state:"hidden"});
    check(await engine.evaluate(el => el === document.activeElement), "hover details stole focus before row eviction");
    check(await page.locator("dialog[open]").count() === 0, "evicted row left a modal behind");
    results.push("hover details close on automatic row eviction while preserving the existing keyboard focus");
  };

  const scenarios={controls,history,pagination,live,recovery};
  const selected=detailUI.internalLogsScenario ?? "all";
  if (selected !== "all" && !Object.hasOwn(scenarios,selected)) throw Error(`Unknown internal logs scenario: ${selected}`);
  try {
    for (const scenario of selected === "all" ? Object.values(scenarios) : [scenarios[selected]]) await scenario();
    return {passed:results.length,results};
  } catch (cause) { throw Error(`After ${results.length} checks (${results.at(-1)??"startup"}) in ${selected}: ${String(cause)}`); }
}
