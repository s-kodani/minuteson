async (page, capturePath = path => path) => {
  const results = [], check = (ok, msg) => { if (!ok) throw Error(msg); };
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch(); const body = (await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/startup-attention-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  const boot = async (variant = "blocked", dedicated = false) => { await page.goto(`http://127.0.0.1:1420/?fixture=${variant}${dedicated ? "&window=startup-attention" : ""}`); await page.getByRole("main", { name: "起動時の案内", exact: true }).waitFor(); await page.waitForFunction(() => window.startupFixture?.calls.length > 0); };
  const quit = () => page.getByRole("button", { name: "このアプリを終了", exact: true });
  const openSalvage = () => page.getByRole("button", { name: "データ復元", exact: true });
  const reload = () => page.getByRole("button", { name: "起動できるか再確認", exact: true });
  const reloadIsSpaced = () => reload().evaluate(button => {
    const screen = button.closest(".startup-attention");
    const text = screen.querySelector(".startup-attention-error") ?? screen.querySelector(".startup-attention-lead");
    const textBounds = text.getBoundingClientRect();
    const buttonBounds = button.getBoundingClientRect();
    return buttonBounds.top - textBounds.bottom >= 12 && Math.abs(buttonBounds.left - textBounds.left) < 1;
  });
  const commands = () => page.evaluate(() => window.startupFixture.calls.map(c => c.command));
  const noWorkspace = async () => { check(await page.locator(".app-layout,.mini-shell").count() === 0, "workspace/provider UI mounted while denied"); check((await commands()).every(c => ["get_startup_report", "get_startup_reset_state", "quit_blocked_startup"].includes(c)), "attention invoked a normal desktop command"); };

  await boot(); await page.getByText("保存先に問題があり、アプリを開けませんでした。", { exact: true }).waitFor();
  check((await page.getByLabel("アプリを開けなかった原因", { exact: true }).innerText()).includes("<img src=x onerror=alert(1)>"), "raw cause was not retained as text");
  check(await page.locator(".startup-attention img").count() === 0, "diagnostic rendered as HTML"); await noWorkspace();
  check(await page.getByRole("button", { name: "保持した診断だけを再読込", exact: true }).count() === 0, "refresh control remains");
  check(await openSalvage().isEnabled(), "a confirmed storage fault did not offer partial salvage");
  check(await page.locator(".startup-attention h1,.startup-attention dl,.startup-attention .status-card").count() === 0, "verbose diagnostic panels remain");
  check(await page.locator(".startup-attention-cause").evaluate(element => {
    const heading = element.querySelector("h2").getBoundingClientRect(), message = element.querySelector("pre").getBoundingClientRect();
    return message.top >= heading.bottom && Math.abs(heading.left - message.left) < 1;
  }), "cause label and message are not vertically aligned");
  const actions = page.locator(".startup-attention-actions > button");
  check((await actions.allTextContents()).join("|") === "起動をやり直す|データ復元|データを初期化|このアプリを終了", "wrong startup action order");
  for (const width of [1200, 640, 360, 320]) {
    await page.setViewportSize({ width, height: 800 });
    const bounds = await actions.evaluateAll(buttons => buttons.map(button => { const rect = button.getBoundingClientRect(); return { top: rect.top, right: rect.right, left: rect.left }; }));
    check(bounds.every(rect => Math.abs(rect.top - bounds[0].top) < 1 && rect.left >= 0 && rect.right <= width + 1), `actions are not one reachable row at ${width}px`);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `startup overflows at ${width}px`);
  }
  await page.setViewportSize({ width: 1360, height: 900 });
  await boot("blocked-clean"); await openSalvage().waitFor();
  await page.screenshot({ path: capturePath("startup-attention-simple.png"), fullPage: true });
  results.push("one lead, vertical cause, four actions on one row; diagnostics remain plain text without refresh");

  await page.evaluate(()=>window.startupFixture.emitResidency("residency-monitor-status","合成監視障害"));
  await page.clock.runFor(300);
  check(await page.getByRole("alert").filter({hasText:"常駐監視: 合成監視障害"}).count()===0,"internal monitor notified during startup");
  await page.evaluate(()=>window.startupFixture.emitResidency("residency-monitor-status",null));
  await page.getByRole("alert").filter({hasText:"常駐監視: 合成監視障害"}).waitFor({state:"hidden"});
  check(await page.getByLabel("アプリを開けなかった原因", { exact: true }).count() === 1 && await quit().isEnabled(), "monitor event erased original blocked report");
  await noWorkspace();
  results.push("internal monitor events do not erase or duplicate the startup cause");

  for (const [variant, reads] of [["read-fail", 4], ["invalid", 1], ["malformed-salvage-field", 1]]) {
    await boot(variant);
    await page.getByRole("alert").filter({ hasText: "アプリを開ける状態か確認できませんでした。" }).waitFor();
    check(await page.getByText(/起動報告|起動診断|矛盾/).count() === 0, "internal startup details reached the user");
    await noWorkspace(); check(await quit().count() === 0, "unverified admission enabled special failure exit");
    check(await openSalvage().count() === 0, "unverified admission offered partial salvage");
    check((await commands()).filter(command => command === "get_startup_report").length === reads, "startup did not separate transport retries from invalid evidence");
    if (variant === "read-fail" || variant === "invalid") {
      check(await reloadIsSpaced(), "report text and reload button need vertical spacing and a shared left edge");
      await page.screenshot({ path: capturePath(`startup-${variant}.png`), fullPage: true });
      for (const width of [360, 320]) {
        await page.setViewportSize({ width, height: 740 });
        check(await reloadIsSpaced(), `report text and reload button are too close or misaligned at ${width}px`);
        check(await reload().evaluate(button => {
          const icon = button.querySelector("svg").getBoundingClientRect(), label = button.querySelector("span").getBoundingClientRect();
          const bounds = button.getBoundingClientRect();
          return Math.abs((icon.top + icon.bottom) - (label.top + label.bottom)) < 2
            && icon.right < label.left && bounds.left >= 0 && bounds.right <= innerWidth
            && button.contains(document.elementFromPoint((bounds.left + bounds.right) / 2, (bounds.top + bounds.bottom) / 2));
        }), `startup retry icon and label do not fit one line at ${width}px`);
        if (variant === "invalid" && width === 360) await page.screenshot({ path: capturePath("startup-reload-narrow.png"), fullPage: true });
      }
      await page.setViewportSize({ width: 1360, height: 900 });
    }
    await page.evaluate(() => { window.startupFixture.readFail = false; window.startupFixture.report = { schemaVersion: 1, phase: "blocked", failure: { stage: "storage_admission", message: "保存先が接続されていません。", messageTruncated: false, meetingRoot: null, configRoot: null } }; });
    await reload().click();
    await page.getByLabel("アプリを開けなかった原因", { exact: true }).filter({ hasText: "保存先が接続されていません。" }).waitFor();
    check(await openSalvage().count() === 0, "legacy report offered partial salvage without native permission");
  }
  await boot("transient-read-fail"); await quit().waitFor();
  check((await commands()).filter(command => command === "get_startup_report").length === 3, "transient startup reads did not recover automatically");
  results.push("transport failures retry at most three times and a manual read can recover; invalid replies never open normal operations");

  const loadingLayout = () => page.locator(".startup-loading-group:visible").evaluate(group => {
    const bounds = group.getBoundingClientRect(), label = group.querySelector("p"), track = group.querySelector('[role="progressbar"]');
    const text = label.getBoundingClientRect(), bar = track.getBoundingClientRect();
    const range = document.createRange(); range.selectNodeContents(label); const ink = range.getBoundingClientRect();
    const content = document.querySelector(".windows-desktop-content")?.getBoundingClientRect() ?? { left:0, top:0, width:innerWidth, height:innerHeight };
    const oneLine = ink.height < parseFloat(getComputedStyle(label).fontSize) * 2;
    return Math.abs(bounds.left + bounds.width / 2 - content.left - content.width / 2) < 2 &&
      Math.abs(bounds.top + bounds.height / 2 - content.top - content.height / 2) < 2 &&
      Math.abs(bar.width - text.width) < 1 && (!oneLine || Math.abs(bar.width - ink.width) < 1.5) &&
      Math.abs(bar.top - text.bottom - 20) < 1 && bar.height === 6 &&
      bounds.left >= 0 && bounds.right <= innerWidth && bounds.top >= 0 && bounds.bottom <= innerHeight &&
      document.documentElement.scrollWidth <= innerWidth && document.documentElement.scrollHeight <= innerHeight &&
      !track.hasAttribute("aria-valuenow") && !track.textContent.trim();
  });
  const waitForProgressFill = () => page.waitForFunction(() => {
    const track = [...document.querySelectorAll(".startup-loading-progress")].find(element => element.getClientRects().length);
    if (!track) return false;
    const bar = track.getBoundingClientRect(), fill = track.querySelector("span").getBoundingClientRect();
    return Math.min(bar.right, fill.right) - Math.max(bar.left, fill.left) >= bar.width / 4;
  });
  for (const [variant, name] of [["starting", "MinutesOn"], ["windows-starting", "MinutesOn QA"]]) {
    await page.goto(`http://127.0.0.1:1420/?fixture=${variant}&productName=${encodeURIComponent(name)}`);
    await page.getByRole("status").getByText(`${name}を起動しています…`, { exact:true }).waitFor();
    const progress = page.getByRole("progressbar", { name:"起動処理中", exact:true });
    await progress.waitFor();
    for (const width of [1360, 320, 240]) {
      await page.setViewportSize({ width, height:800 });
      check(await loadingLayout(), `startup label/bar lost intrinsic width, center, spacing or viewport fit: ${name}/${width}`);
    }
    const fill = progress.locator("span");
    check(await fill.evaluate(element => getComputedStyle(element).animationName === "startup-loading-sweep"), "indeterminate startup did not use CSS animation");
    await page.emulateMedia({ reducedMotion:"reduce" });
    check(await fill.evaluate(element => {
      const bar = element.parentElement.getBoundingClientRect(), bounds = element.getBoundingClientRect();
      return getComputedStyle(element).animationName === "none" && bounds.width > 0 && bounds.left >= bar.left && bounds.right <= bar.right;
    }), "reduced-motion startup has no visible static fill");
    await page.emulateMedia({ reducedMotion:"no-preference" });
    await page.setViewportSize({ width:variant === "starting" ? 1360 : 320, height:800 });
    await waitForProgressFill();
    await page.screenshot({ path:capturePath(variant === "starting" ? "startup-progress-1360.png" : "startup-progress-qa-320.png") });
    check((await commands()).every(command => command === "get_startup_report"), "startup progress introduced a native command");
  }
  results.push("startup label and CSS progress form one natural-width group at desktop, narrow and small-window widths; QA name and reduced motion remain supported");

  for (const [role, path, width, height] of [["main", "/src/main.tsx", 1360, 900], ["recorder", "/src/recorder-panel.tsx", 227, 101], ["recording-confirmation", "/src/recording-confirmation.tsx", 420, 280]]) {
    const moduleURL = url => url.pathname === path;
    const { promise: heldModule, resolve: releaseModule } = Promise.withResolvers();
    const continuations = [];
    const holdModule = route => { const continuation = heldModule.then(() => route.continue()); continuations.push(continuation); return continuation; };
    await page.route(moduleURL, holdModule);
    try {
      await page.setViewportSize({ width, height });
      const requested = page.waitForRequest(request => moduleURL(new URL(request.url())));
      await page.goto(`http://127.0.0.1:1420/?fixture=windows-starting&window=${role}&productName=MinutesOn%20QA`);
      await page.waitForFunction(() => !!window.startupFixture);
      if (role === "main") {
        await page.evaluate(() => { window.startupFixture.report = { schemaVersion:1, phase:"ready", failure:null, platform:"windows" }; });
        await page.clock.runFor(250);
      }
      await requested;
      await page.getByRole("progressbar", { name:"起動処理中", exact:true }).waitFor();
      await page.locator(".startup-loading:visible").getByText("MinutesOn QAを起動しています…", { exact:true }).waitFor();
      check(await loadingLayout() && await page.locator(".app-layout,.mini-shell").count() === 0, `bootstrap Suspense did not fit its shared startup group: ${role}/${width}x${height}`);
      if (role === "recorder") { await waitForProgressFill(); await page.screenshot({ path:capturePath("startup-progress-qa-recorder-227.png") }); }
    } finally {
      releaseModule(); await Promise.all(continuations); await page.unroute(moduleURL, holdModule);
      await page.goto("about:blank");
    }
  }
  results.push("bootstrap Suspense retains the shared startup group for real main, 227x101 recorder and 420x280 confirmation module loads");

  for (const phase of ["ready", "blocked"]) {
    await page.goto(`http://127.0.0.1:1420/?fixture=transition-${phase}`);
    await page.getByRole("main", { name: "起動確認", exact: true }).waitFor();
    const center = await page.locator(".startup-loading-group:visible").evaluate(element => {
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2, width: innerWidth, height: innerHeight };
    });
    check(Math.abs(center.x - center.width / 2) < 2 && Math.abs(center.y - center.height / 2) < 2,
      "startup message is not centered in the viewport");
    check(await page.getByRole("heading", { name: "保存先の起動確認", exact: true }).count() === 0,
      "normal pending startup showed the diagnostic page");
    check(await page.getByRole("main", { name: "通常画面", exact: true }).count() === 0,
      "workspace mounted before confirmed admission");
    if (phase === "ready") await page.getByRole("main", { name: "通常画面", exact: true }).waitFor();
    else { await quit().waitFor(); await noWorkspace(); }
    check(await page.getByRole("progressbar", { name:"起動処理中", exact:true }).count() === 0, "startup progress remained after ready/blocked settled");
    const settled = (await commands()).length;
    await page.clock.runFor(450);
    check((await commands()).length === settled && settled >= 2,
      "startup did not reread pending memory or kept polling a settled result");
    check((await commands()).every(c => ["get_startup_report", "get_startup_reset_state"].includes(c)), "pending startup invoked storage work");
  }
  results.push("pending startup reads memory until ready/blocked, then stops; normal startup needs no confirmation");

  await boot("selection"); await page.getByText("起動設定にアクセスできませんでした。読み取り権限を確認してください。", { exact: true }).waitFor(); await noWorkspace();
  check(await openSalvage().count() === 0, "configuration access fault offered partial salvage");
  await page.screenshot({ path: capturePath("startup-selection-blocked.png"), fullPage: true });
  results.push("selection refusal has correct stage and does not invent a selected root");

  await boot("blocked", true); await quit().waitFor(); await page.evaluate(() => { window.startupFixture.quitFail = true; window.startupFixture.delay = true; });
  await quit().click(); check(await actions.evaluateAll(buttons => buttons.every(button => button.disabled)), "quit response wait allowed concurrent recovery");
  await page.evaluate(() => { window.startupFixture.pendingQuit(); });
  await page.getByRole("alert").filter({ hasText: "アプリを終了できたか確認できませんでした" }).waitFor();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 1, "lost quit automatically replayed"); await noWorkspace();
  await page.evaluate(() => { window.startupFixture.delay = false; window.startupFixture.quitFail = false; }); await quit().click();
  check((await commands()).filter(c => c === "quit_blocked_startup").length === 2, "explicit quit retry missing");
  results.push("failure-only quit has single admission and explicit retry after response loss");

  await boot("windows"); await quit().waitFor();
  await page.getByText("原因を解消してから、起動をやり直してください。", { exact: true }).waitFor();
  check(await openSalvage().count() === 0, "temporary storage lock offered partial salvage");
  const retry = page.getByRole("button", { name: "起動をやり直す", exact: true });
  check(await retry.isEnabled(), "temporary storage lock did not provide startup retry");
  await page.screenshot({ path: capturePath("startup-windows-lock.png"), fullPage: true });
  await retry.click();
  check((await commands()).filter(command => command === "retry_startup").length === 1, "temporary startup retry did not reach native code once");

  await boot("blocked-clean"); await openSalvage().waitFor();
  await page.evaluate(() => { window.startupFixture.retryFail = true; });
  await retry.click();
  await page.getByRole("alert").filter({ hasText: "再起動できたか確認できませんでした" }).waitFor();
  check(await openSalvage().count() === 0, "an uncertain restart allowed a second recovery action");
  check(await reloadIsSpaced(), "startup lead text and reload button need vertical spacing");
  await page.screenshot({ path: capturePath("startup-restart-unconfirmed.png"), fullPage: true });
  await page.setViewportSize({ width: 360, height: 740 });
  check(await reloadIsSpaced(), "wrapped startup lead text and reload button need vertical spacing");
  await page.setViewportSize({ width: 1360, height: 900 });

  await boot("blocked-clean");
  const dialog = page.getByRole("dialog", { name: "データ復元", exact: true });
  await openSalvage().click();
  await dialog.getByText(/読み取れるデータを新しい保存先に復元できます/).waitFor();
  check(await dialog.getByRole("radio").count() === 0, "restoration exposed initialization or archive choices");
  check(!(await commands()).some(command => ["open_salvage", "get_startup_reset_preview", "start_startup_reset"].includes(command)), "opening restoration ran an operation");
  await page.screenshot({ path: capturePath("startup-restoration-intro.png"), fullPage: true });
  await dialog.getByRole("button", { name: "再確認", exact: true }).click();
  await dialog.getByRole("status").filter({ hasText: "保存先フォルダーの再確認を行いました。" }).waitFor();
  await page.screenshot({ path: capturePath("startup-restoration-recheck.png"), fullPage: true });
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "データ復元");
  await openSalvage().click();
  await dialog.getByRole("button", { name: "復元内容の確認に進む", exact: true }).click();
  await dialog.getByRole("button", { name: "復元先の親フォルダーを選択", exact: true }).waitFor();
  check((await commands()).filter(command => command === "open_salvage").length === 1, "restoration entered duplicate workflows");
  check(!(await commands()).some(command => ["get_startup_reset_preview", "start_startup_reset", "commit_salvage"].includes(command)), "restoration touched legacy initialization or committed prematurely");
  results.push("startup offers one restoration entry, restores focus on cancellation, and prepares restoration without calling initialization");

  await boot("blocked-clean");
  const initialize = page.getByRole("button", { name: "データを初期化", exact: true });
  await initialize.click();
  const initialization = page.getByRole("dialog", { name: "データを初期化", exact: true });
  await initialization.waitFor();
  check(await initialization.getByRole("radio").count() === 2, "initialization lost its separate archive choices");
  const finish = initialization.getByRole("button", { name: "初期化して再起動", exact: true });
  check(await finish.isDisabled(), "initialization skipped a required archive target");
  await initialization.getByRole("button", { name: "退避先を選択", exact: true }).click();
  await initialization.getByText("/safe/初期化前の退避", { exact: true }).waitFor();
  await page.evaluate(() => { window.startupFixture.startReplyFail = true; });
  await finish.click();
  await initialization.getByRole("status").filter({ hasText: "旧データを退避しています" }).waitFor();
  check(await initialization.getByRole("button", { name: "キャンセル", exact: true }).isDisabled(), "active initialization could be dismissed as cancelled");
  check((await commands()).filter(command => command === "start_startup_reset").length === 1, "progress polling replayed initialization");
  await page.keyboard.press("Escape");
  check(await initialization.isVisible(), "Escape hid active initialization");
  await page.evaluate(() => {
    Object.assign(window.startupFixture.resetState, { busy: false, message: "データの初期化を完了できませんでした", error: "元データを保持しました" });
  });
  await initialization.getByRole("alert").filter({ hasText: "元データを保持" }).waitFor();
  await initialization.getByRole("button", { name: "キャンセル", exact: true }).click();
  const restart = page.locator(".startup-reset-actions > button");
  await restart.waitFor();
  for (const width of [900, 360, 320]) {
    await page.setViewportSize({ width, height: 740 });
    const bounds = await page.evaluate(() => {
      const above = document.querySelector(".startup-attention-actions").getBoundingClientRect();
      const button = document.querySelector(".startup-reset-actions > button").getBoundingClientRect();
      return { gap: button.top - above.bottom, left: button.left, right: button.right };
    });
    check(bounds.gap >= 16 && bounds.left >= 0 && bounds.right <= width, "failed initialization's restart fell outside the action layout");
  }
  await restart.click();
  check((await commands()).filter(command => command === "retry_startup").length === 1, "initialization notice did not expose restart");
  await boot("reset-stopped-empty"); await restart.waitFor();
  check(await page.evaluate(() => document.querySelector(".startup-reset-actions > button").getBoundingClientRect().top - document.querySelector(".startup-attention-actions").getBoundingClientRect().bottom >= 16),
    "restart spacing depended on having a result message");
  await boot("reset-running");
  await page.getByRole("status").filter({ hasText: "旧データを退避しています" }).waitFor();
  check(!(await commands()).includes("start_startup_reset"), "reconnection restarted initialization");
  results.push("initialization retains archive selection, reports progress and errors, and does not replay after reply loss");

  await boot("long"); await page.setViewportSize({ width: 360, height: 740 }); await page.emulateMedia({ colorScheme: "dark" });
  await page.getByRole("heading", { name: "原因（一部のみ表示）", exact: true }).waitFor();
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), "long diagnosis overflows small width");
  check(await page.getByLabel("アプリを開けなかった原因", { exact: true }).evaluate(e => e.scrollHeight > e.clientHeight), "long cause cannot scroll");
  await quit().scrollIntoViewIfNeeded(); check(await quit().isVisible(), "long diagnosis hides reachable exit");
  await page.screenshot({ path: capturePath("startup-attention-360-dark.png"), fullPage: true }); await noWorkspace(); await page.emulateMedia({ colorScheme: "light" });
  results.push("long truncated diagnosis scrolls, wraps and leaves exit reachable at 360px in dark mode");

  await page.setViewportSize({ width: 1360, height: 900 });
  await boot("replaced-registration");
  await page.getByText("登録したフォルダーまたは会議データのファイルが現在のものと一致しません。保存先を確認してください。", { exact: true }).waitFor();
  check(await openSalvage().count() === 0, "an unrecognised registered store offered restoration");
  await page.screenshot({ path: capturePath("startup-replaced-registration.png"), fullPage: true });
  results.push("a replaced registered folder stays in startup diagnostics without offering an unusable restore");

  await boot("partial-salvage");
  const salvage = page.getByRole("dialog", { name: "データ復元", exact: true });
  await salvage.getByRole("button", { name: "復元先の親フォルダーを選択", exact: true }).click();
  await salvage.getByRole("button", { name: "復元できる内容を確認", exact: true }).click();
  await salvage.getByRole("cell", { name: "未検査", exact: true }).waitFor();
  await salvage.getByText(/元の会議総数を確認できませんでした/).waitFor();
  await salvage.getByRole("status").filter({ hasText: "空き容量が不足しています" }).waitFor();
  const commit = salvage.getByRole("button", { name: "復元して保存先を切り替える", exact: true });
  check(await commit.isDisabled(), "insufficient capacity allowed publication");
  await page.screenshot({ path: capturePath("salvage-capacity-change.png"), fullPage: true });
  await page.evaluate(() => { window.startupFixture.capacityFreed = true; });
  await salvage.getByRole("button", { name: "空き容量を再確認", exact: true }).click();
  await page.waitForFunction(() => Array.from(document.querySelectorAll("dialog[open] button")).some(button => button.textContent === "復元して保存先を切り替える" && !button.disabled));
  check(await page.locator(".app-layout").count() === 0 && !(await commands()).includes("commit_salvage"), "capacity recheck published data or enabled workspace");
  results.push("dedicated salvage reports uninspected data, blocks insufficient capacity and refreshes the fixed preview without publishing");

  await boot("partial-salvage");
  await salvage.getByRole("button", { name: "復元先の親フォルダーを選択", exact: true }).click();
  const inspect = salvage.getByRole("button", { name: "復元できる内容を確認", exact: true });
  await inspect.click();
  await salvage.getByRole("status").filter({ hasText: "空き容量が不足しています" }).waitFor();
  const changeDestination = salvage.getByRole("button", { name: "復元先を変更", exact: true });
  await page.evaluate(() => { window.startupFixture.folderCancelled = true; });
  await changeDestination.click();
  check(await commit.isDisabled(), "cancelling the folder dialog invalidated the capacity hold");
  await page.evaluate(() => { window.startupFixture.folderCancelled = false; });
  await changeDestination.click();
  await salvage.getByText("/safe/外付けドライブ/新しい保存先", { exact: true }).waitFor();
  check(await commit.count() === 0, "a new destination retained the previous confirmation");
  await page.screenshot({ path: capturePath("salvage-new-destination.png"), fullPage: true });
  await inspect.click();
  await commit.waitFor();
  check(await commit.isEnabled(), "the separately verified destination could not be used");
  check(!(await commands()).some(command => ["commit_salvage", "retry_startup", "leave_salvage"].includes(command)), "destination change ended the restore session or published data");
  await page.screenshot({ path: capturePath("salvage-rechecked-destination.png"), fullPage: true });
  results.push("another destination can be selected and verified within the same restore session");

  await boot("partial-salvage");
  await salvage.getByRole("button", { name: "復元先の親フォルダーを選択", exact: true }).click();
  await page.evaluate(() => { Object.assign(window.startupFixture, { capacityFreed: true, holdWork: true }); });
  const showProgress = async (stage, unit, completed, total, label, filename) => {
    await page.evaluate(progress => { window.startupFixture.salvage.progress = progress; }, { stage, unit, completed, total });
    await salvage.getByText(label, { exact: true }).waitFor();
    await page.screenshot({ path: capturePath(filename), fullPage: true });
  };
  await inspect.click();
  await showProgress("copying_salvage_database", "bytes", 1024 * 1024, 2 * 1024 * 1024,
    "元の会議データを一時フォルダーにコピーしています。", "salvage-copying-database.png");
  await page.evaluate(() => window.startupFixture.finishWork());
  await commit.click();
  await showProgress("building_meetings", "meetings", 1, 2,
    "新しい保存先に会議データを保存しています。", "salvage-saving.png");
  await showProgress("verifying_salvage", "items", 0, null,
    "復元したデータを読み込めるか確認しています。", "salvage-verifying-new-database.png");
  await page.evaluate(() => window.startupFixture.finishWork());
  results.push("restore progress names the files being copied and the work in the selected destination");

  await page.goto("http://127.0.0.1:1420/?fixture=ready-salvage-edit");
  await openSalvage().click();
  const restartWithEdits = salvage.getByRole("button", { name: "再起動して復元に進む", exact: true });
  await restartWithEdits.click();
  const exportEdits = salvage.getByRole("button", { name: "未保存の入力を別のファイルに保存", exact: true });
  const exportError = text => salvage.getByRole("alert").filter({ hasText: text });
  await exportEdits.waitFor();
  await page.evaluate(() => { window.startupFixture.exportFails = true; });
  await exportEdits.click();
  await exportError("保存先にファイルを書き込めませんでした").waitFor();
  check(!(await commands()).includes("finish_main_close"), "a failed save restarted with unsaved edits");
  await page.screenshot({ path: capturePath("salvage-edit-save-failed.png"), fullPage: true });
  await page.evaluate(() => { window.startupFixture.exportFails = false; });
  await exportEdits.click();
  await salvage.getByRole("status").filter({ hasText: "ファイルへの保存を取り消しました" }).waitFor();
  await page.screenshot({ path: capturePath("salvage-edit-save-cancelled.png"), fullPage: true });
  await restartWithEdits.click();
  await exportError("編集中の内容を保存できませんでした").waitFor();
  check(!(await commands()).includes("finish_main_close"), "cancelling save let the user restart with unsaved edits");
  await page.screenshot({ path: capturePath("salvage-edit-save-offered.png"), fullPage: true });
  await page.evaluate(() => { window.startupFixture.exportSaved = true; });
  await exportEdits.click();
  const saved = salvage.getByRole("status").filter({ hasText: "未保存の入力を別のファイルに保存しました" });
  await saved.waitFor();
  await page.screenshot({ path: capturePath("salvage-edit-save-confirmed.png"), fullPage: true });
  await page.evaluate(() => { window.startupFixture.liveEdit = "更新した議事録"; });
  await restartWithEdits.click();
  await exportError("今の入力がファイルに保存されていることを確認できません").waitFor();
  check(!(await commands()).includes("finish_main_close"), "an older saved file allowed restart after inputs changed");
  await page.screenshot({ path: capturePath("salvage-edit-save-changed.png"), fullPage: true });
  await exportEdits.click();
  await saved.waitFor();
  await restartWithEdits.click();
  await salvage.getByRole("status").filter({ hasText: "再起動を待っています" }).waitFor();
  check((await commands()).filter(command => command === "finish_main_close").length === 1, "confirmed export did not allow the requested restart");
  check(await page.evaluate(() => window.startupFixture.calls.some(call => call.command === "export_salvage_edits" && JSON.parse(call.args.contents).edits[0].input.markdown === "更新した議事録")), "native export did not receive the updated input");
  results.push("only a confirmed native save of the current inputs allows a restore restart");

  await boot("ready", true); await page.getByRole("link", { name: "通常画面を開く", exact: true }).waitFor();
  check(await quit().count() === 0 && (await commands()).length === 1, "ready attention used failure-only exit or workspace command");
  results.push("ready read-only attention never uses failure exit; normal navigation requires explicit input");
  return { passed: results.length, results };
}
