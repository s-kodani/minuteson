async page => {
  await page.getByRole("button", { name: "会議A", exact: true }).click();
  const summary = page.getByText(/^用語辞書の適用情報（未適用/);
  const text = page.getByRole("searchbox", { name: "発言を検索" });
  await text.waitFor();
  if (await summary.count()) throw Error("Whisper の結果に適用情報が表示されました");

  await page.evaluate(() => window.monBrowserFixture.setIgnoredHotwords([
    { term: "A/B", reason: "reservedSyntax" },
    { term: "C#", reason: "reservedSyntax" },
    { term: "New York", reason: "whitespace" },
    { term: "𠮷", reason: "outsideVocabulary" },
    { term: "旧用語", reason: "unknown" },
  ]));
  await summary.waitFor();
  if (!(await summary.innerText()).includes("5 件")) throw Error("未適用の件数が一致しません");
  await page.setViewportSize({ width: 400, height: 900 });
  await summary.focus();
  await page.keyboard.press("Enter");
  const details = summary.locator("..");
  for (const reason of [/A\/B.*予約文字/, /C#.*予約文字/, /New York.*空白/, /𠮷.*語彙/, /旧用語.*理由不明/]) {
    await details.getByRole("listitem").filter({ hasText: reason }).waitFor();
  }
  if (await details.evaluate(element => element.scrollWidth > element.clientWidth)) throw Error("適用情報が横にはみ出しています");

  await page.evaluate(() => window.monBrowserFixture.setIgnoredHotwords([]));
  await summary.waitFor({ state: "detached" });
  return { passed: 3, results: ["Whisper で非表示", "未適用5件と理由を表示", "除外のない結果で非表示"] };
}
