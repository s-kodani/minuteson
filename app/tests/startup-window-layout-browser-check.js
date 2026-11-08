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
  return results;
}
