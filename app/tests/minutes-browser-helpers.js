// Regression-driver helper only; not imported by the product bundle.
export function minutesFixtureIntegrity(data, baseline) {
  const payload = data.drafts.a?.payload;
  return {
    blocks: payload?.blocks.length ?? 0,
    items: payload?.items.length ?? 0,
    evidence: payload?.evidence.length ?? 0,
    undoRequests: data.calls.filter(call => call.cmd === "undo_minutes_edit").length,
    originalDraft: JSON.stringify(payload) === baseline.draft,
    originalGenerated: JSON.stringify(data.views.a.content) === baseline.generated,
  };
}

export async function applyMinutesFixtureTheme(page, theme) {
  if (!["light", "dark"].includes(theme)) throw Error("invalid synthetic theme");
  await page.evaluate(theme => {
    if (typeof window.__mon?.changeThemeFromOtherWindow !== "function") throw Error("minutes fixture is required");
    window.__mon.changeThemeFromOtherWindow(theme);
  }, theme);
  await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme);
}
