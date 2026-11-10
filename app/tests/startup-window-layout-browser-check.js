// Production workspace with synthetic IPC. Native monitor/DPI behavior is
// checked separately; this covers usability below the former minimum size.
async (page, capturePath) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const base = new URL(page.url()).origin;
  const nav = page.getByRole("navigation", { name: "メインナビゲーション", exact: true });
  for (const platform of ["macos", "windows"]) {
    await page.goto(`${base}/?resident=${platform}`);
    await page.getByRole("heading", { name: "会議一覧", exact: true }).waitFor();
    await page.evaluate(() => window.monBrowserFixture.seedLongMeetingList());
    for (const size of [{ width: 1280, height: 820 }, { width: 1152, height: 692 }, { width: 905, height: 652 }, { width: 720, height: 512 }]) {
      await page.setViewportSize(size);
      await nav.getByRole("button", { name: "設定", exact: true }).click();
      const mode = page.getByRole("region", { name: "表示設定", exact: true }).getByRole("combobox", { name: "表示モード", exact: true });
      await mode.scrollIntoViewIfNeeded();
      check(await mode.isVisible() && await mode.isEnabled(), `${platform}/${size.width}: settings cannot be reached`);
      await mode.focus();
      check(await mode.evaluate(el => el === document.activeElement), "settings lost keyboard focus");
      await nav.getByRole("button", { name: "会議一覧", exact: true }).click();
      await page.getByRole("button", { name: "再読込", exact: true }).click();
      await page.getByRole("button", { name: "会議A", exact: true }).waitFor();
      const start = page.getByRole("button", { name: "新しい録音", exact: true });
      await start.scrollIntoViewIfNeeded();
      check(await start.isVisible() && await start.isEnabled(), "recording entry cannot be reached");
      const layout = await page.evaluate(() => {
        const content = document.querySelector("main.content").getBoundingClientRect();
        return { overflow: document.documentElement.scrollWidth > innerWidth,
          top: content.top, bottom: content.bottom, height: innerHeight,
          chrome: document.querySelector(".windows-titlebar")?.getBoundingClientRect().height ?? 0 };
      });
      check(!layout.overflow && layout.top >= layout.chrome && layout.bottom <= layout.height + 1, `${platform}/${size.width}: workspace escapes viewport`);
      check(layout.chrome === (platform === "windows" ? 32 : 0), "platform titlebar space is incorrect");
      const last = page.getByRole("button", { name: "一覧スクロール確認 30", exact: true });
      await last.scrollIntoViewIfNeeded();
      const bounds = await last.boundingBox();
      check(bounds.y >= layout.top && bounds.y + bounds.height <= layout.height, "last meeting cannot be scrolled into view");
      if (size.width === 720) await page.screenshot({ path: capturePath(`startup-window-${platform}-720.png`) });
      results.push(`${platform} ${size.width}×${size.height}: navigation, settings focus, recording entry and list scrolling remain accessible`);
    }
  }
  await page.evaluate(() => {
    const fixture = window.monBrowserFixture, invoke = window.__TAURI_INTERNALS__.invoke;
    fixture.restoredLabelIds = ["meeting-0"];
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "record_background_read_failure") { fixture.calls.push({ command, args }); return null; }
      if (command === "list_restored_meeting_ids") {
        if (fixture.failRestoreLabels) throw Error("模擬復元ラベル取得失敗");
        return fixture.restoredLabelIds;
      }
      if (command === "plugin:window|is_maximized") {
        if (fixture.failWindowRead) throw Error("模擬ウィンドウ状態取得失敗");
        return !!fixture.maximized;
      }
      if (command === "plugin:window|toggle_maximize") throw Error("模擬最大化操作失敗");
      return invoke(command, args);
    };
  });
  const reload = page.getByRole("button", { name: "再読込", exact: true });
  const restoredLabel = page.getByText("バックアップから復元", { exact: true });
  await reload.click();
  await restoredLabel.waitFor();
  await page.evaluate(() => { window.monBrowserFixture.failRestoreLabels = true; });
  await reload.click();
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(call => call.args?.source === "restore_labels"));
  check(await restoredLabel.isVisible(), "a failed refresh removed the last confirmed restore label");
  await page.evaluate(() => Object.assign(window.monBrowserFixture, { failRestoreLabels: false, restoredLabelIds: [] }));
  await reload.click();
  await restoredLabel.waitFor({ state: "hidden" });

  const windowControls = page.getByRole("group", { name: "ウィンドウ操作", exact: true });
  await page.evaluate(() => {
    Object.assign(window.monBrowserFixture, { failWindowRead: true, maximized: true });
    return window.monBrowserFixture.emitWindowFocus();
  });
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(call => call.args?.source === "window_maximized"));
  check(await windowControls.getByRole("button", { name: "最大化", exact: true }).isVisible(), "a failed read replaced the last window state");
  await page.evaluate(() => {
    window.monBrowserFixture.failWindowRead = false;
    return window.monBrowserFixture.emitWindowFocus();
  });
  const restoreWindow = windowControls.getByRole("button", { name: "元のサイズに戻す", exact: true });
  await restoreWindow.waitFor();
  await restoreWindow.click();
  await page.getByRole("alert").filter({ hasText: /最大化・復元できません.*模擬最大化操作失敗/ }).waitFor();
  check(await page.evaluate(async () => {
    const { toasts } = await import("/src/toast.tsx");
    return !toasts.getHistorySnapshot().some(notice => /模擬復元ラベル取得失敗|模擬ウィンドウ状態取得失敗/.test(notice.message));
  }), "passive read failures were recorded in notification history");
  results.push("restore labels and Windows state recover after log-only failures, while an explicit window action failure remains visible");
  return results;
}
