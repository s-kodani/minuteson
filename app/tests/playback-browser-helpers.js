// Regression-driver helpers only; never imported by the product bundle.
export async function applyPlaybackFixtureTheme(page, theme) {
  if (!["light", "dark"].includes(theme)) throw Error("invalid synthetic theme");
  await page.evaluate(async theme => {
    const fixture = window.monBrowserFixture;
    if (typeof fixture?.emitUiPreferences !== "function") throw Error("playback fixture is required");
    fixture.uiPreferences ??= { id:"ui_preferences", payload:{ schemaVersion:1, theme:"system", colorTheme:"monochrome" }, lockVersion:0 };
    fixture.uiPreferences.payload.theme = theme;
    fixture.uiPreferences.lockVersion++;
    await fixture.emitUiPreferences();
  }, theme);
  await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme);
}
