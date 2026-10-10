// Regression-driver helper only; not imported by the product bundle.
export async function applyMinutesFixtureTheme(page, theme) {
  if (!["light", "dark"].includes(theme)) throw Error("invalid synthetic theme");
  await page.evaluate(theme => {
    if (typeof window.__mon?.changeThemeFromOtherWindow !== "function") throw Error("minutes fixture is required");
    window.__mon.changeThemeFromOtherWindow(theme);
  }, theme);
  await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme);
}
