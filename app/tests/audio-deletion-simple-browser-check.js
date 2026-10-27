// Full workspace with synthetic IPC only. No native files or user data.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const origin = new URL(page.url()).origin;
  const scenario = async (name, extra = "") => {
    await page.goto(`${origin}/tests/audio-deletion-browser.html?scenario=${name}${extra}`);
    await page.getByRole("heading", { name: "会議一覧", exact: true }).waitFor();
  };
  const menu = async () => page.getByRole("button", { name: "既存会議の操作", exact: true }).click();
  const audio = () => page.getByRole("menuitem", { name: "既存会議の音声だけを削除", exact: true });
  const dialog = () => page.getByRole("dialog", { name: "音声だけを削除", exact: true });
  const status = () => page.locator("#audio-test-status").textContent().then(JSON.parse);
  const waitStatus = (key, expected) => page.waitForFunction(({ key, expected }) => JSON.parse(document.querySelector("#audio-test-status").textContent)[key] === expected, { key, expected });
  const review = async () => { await menu(); await audio().click(); await dialog().waitFor(); };

  await scenario("page-failure");
  await review();
  check(await dialog().getByRole("button").count() === 2 && await dialog().getByRole("checkbox").count() === 0, "new deletion retained detailed consent controls");
  check((await status()).paths === 0, "simple confirmation fetched a path page");
  await dialog().getByRole("button", { name: "キャンセル", exact: true }).click();
  check((await status()).deleted === 0, "cancelling deleted audio");
  results.push("two-button confirmation and cancellation without path reads");

  await page.getByRole("button", { name: "古い削除可否の応答を保留", exact: true }).click();
  await waitStatus("held", true);
  await review();
  await page.locator("dialog[open] .stop-button:not(:disabled)").waitFor();
  await dialog().getByRole("button", { name: "削除", exact: true }).click();
  await waitStatus("deleted", 1);
  await menu();
  check(!await audio().isEnabled(), "completed audio deletion can run again");
  check(await page.locator(".audio-deletion-menu-item").getAttribute("title") === "音声はすでに削除済みです。", "disabled item has no hover explanation");
  await page.getByRole("button", { name: "古い削除可否の応答を返す", exact: true }).click();
  await waitStatus("held", false);
  await menu();
  check(!await audio().isEnabled(), "late availability response enabled deleted audio");
  check(await page.getByRole("menuitem", { name: "既存会議を削除", exact: true }).isEnabled(), "audio deletion blocked whole-meeting deletion");
  check((await status()).paths === 0, "completion fetched a path page");
  results.push("immediate disable, hover help and stale response rejection");

  for (const name of ["deleted", "unknown"]) {
    await scenario(name); await menu();
    check(!await audio().isEnabled(), `${name} audio state allowed deletion`);
    check(await page.getByRole("menuitem", { name: "既存会議を削除", exact: true }).isEnabled(), `${name} audio state blocked meeting deletion`);
    results.push(`${name} startup state disables only audio deletion`);
  }
  await scenario("unregistered", "&resident=macos");
  await review();
  await page.locator("dialog[open] .stop-button:not(:disabled)").waitFor();
  await dialog().getByRole("button", { name: "削除", exact: true }).click();
  await waitStatus("deleted", 1);
  check((await status()).actions.join(",") === "delete_including_unregistered", "unregistered audio was not included in the native request");
  check((await status()).paths === 0, "macOS startup mock fetched a path page");
  results.push("macOS startup mock includes classified unregistered audio with one confirmation");

  await scenario("recovery");
  const recovery = page.getByRole("dialog", { name: "音声だけの削除を確認", exact: true });
  await recovery.getByText("1〜3 / 3 件", { exact: true }).waitFor();
  check(await recovery.getByRole("checkbox").count() === 1, "recovery lost its existing confirmation");
  check(!await recovery.getByRole("button", { name: "照合して音声削除を続行", exact: true }).isEnabled(), "recovery bypassed consent");
  results.push("interrupted deletion retains detailed confirmation and saved policy");
  return { passed: results.length, results };
}
