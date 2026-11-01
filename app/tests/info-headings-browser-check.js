// Production screens with synthetic IPC; explanations never execute native actions.
async (page, capturePath) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const info = label => page.getByRole("button", { name: `${label}の説明を開く`, exact: true });
  const popup = label => page.getByRole("dialog", { name: label, exact: true });
  const visit = async (label, text) => {
    const button = info(label);
    await button.click(); await popup(label).waitFor();
    check((await popup(label).innerText()).includes(text), `${label}: explanation lost`);
    check(await button.evaluate(e => !e.closest("label")), `${label}: info nested inside form label`);
    check(await popup(label).locator("p p").count() === 0, `${label}: invalid nested paragraphs`);
    await page.keyboard.press("Escape"); await popup(label).waitFor({ state: "hidden" });
  };
  await page.getByRole("heading", { name: "会議一覧", exact: true }).waitFor();
  check(await page.locator(".brand-name").innerText() === "MinutesOn", "formal application name missing from sidebar");
  check(await page.locator(".brand").evaluate(el => el.scrollWidth <= el.clientWidth), "formal application name overflows sidebar");
  check(await page.title() === "MinutesOn — 会議記録", "formal application name missing from document title");
  check(!(await page.locator("body").innerText()).includes("端末内で会議を記録"), "sidebar tagline remains");
  check(await page.getByText("YOUR WORKSPACE", { exact: true }).count() === 0, "workspace eyebrow remains");
  check(await page.getByText("会話を記録して、次のアクションにつなげる。", { exact: true }).count() === 0, "meeting tagline remains");
  check(await page.getByRole("heading", { name: "会議一覧", exact: true }).isVisible(), "meeting list title missing");
  for (const [screen, text] of [["ジョブ一覧", "処理中・待機中"], ["ボイス一覧", "声の特徴量"], ["プロンプト", "共通用語"], ["内部ログ", "最大30日"], ["設定", "録音環境"]]) {
    await page.locator(".sidebar").getByRole("button", { name: screen, exact: true }).click();
    await page.getByRole("heading", { name: screen, exact: true }).waitFor();
    check(await page.locator(".content > .lead, .page-heading > .lead").count() === 0, `${screen}: inline description remains`);
    await visit(screen, text);
    const heading = page.getByRole("heading", { name: screen, exact: true });
    const h = await heading.boundingBox(), b = await info(screen).boundingBox();
    check(b.x >= h.x + h.width - 1 && Math.abs(b.y + b.height / 2 - h.y - h.height / 2) < 2, `${screen}: info is not beside title`);
    results.push(`${screen}: title retains an adjacent info icon and full explanation`);
    if (screen === "内部ログ") {
      await page.getByRole("tab", { name: "エンジンログ", exact: true }).click();
      await visit("エンジンログ", "stderrは必ずしもエラーではありません");
    }
    if (screen === "プロンプト") {
      await visit("議事録生成プロンプト", "次の新規生成");
      await visit("作成用プロンプト", "文字起こしから");
      await visit("統合用プロンプト", "長い会議を分割");
      check(await page.getByRole("textbox", { name: "作成用プロンプト", exact: true }).count() === 1, "prompt label association lost");
    }
  }
  await info("設定").click(); await popup("設定").waitFor();
  await page.waitForTimeout(1200);
  check(await popup("設定").isVisible(), "background state polling dismissed help");
  await page.locator(".sidebar").getByRole("button", { name: "会議一覧", exact: true }).click();
  check(await page.locator(".info-popover:popover-open").count() === 0, "navigation left old help open");
  results.push("background updates retain pinned help; navigation dismisses it");
  await page.locator(".sidebar").getByRole("button", { name: "設定", exact: true }).click();
  for (const [label, text] of [["話者分離のCPU使用量", "次に開始"], ["自動の話者統合", "別人"], ["登録した声の自動適用", "類似度は確率ではありません"], ["未検出発言の話者補完", "次に開始する補完"]]) await visit(label, text);
  for (const theme of ["light", "dark"]) for (const width of [360, 960]) {
    await page.setViewportSize({ width, height: 720 });
    await page.evaluate(value => document.documentElement.dataset.theme = value, theme);
    await info("未検出発言の話者補完").click(); await popup("未検出発言の話者補完").waitFor();
    const box = await popup("未検出発言の話者補完").boundingBox();
    check(box.x >= 8 && box.x + box.width <= width - 8 && box.y >= 8 && box.y + box.height <= 712, "long help exceeds viewport");
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "settings overflow horizontally");
    await page.screenshot({ path: capturePath(`heading-help-${theme}-${width}.png`) });
    await page.keyboard.press("Escape");
  }
  check(await page.locator("label .info-trigger").count() === 0, "info button nested in label");
  results.push("setting explanations remain available with disabled inputs and fit narrow/wide light/dark screens");
  return { passed: results.length, results };
}
