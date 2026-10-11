// Production rich editor with synthetic IPC. No native files or audio.
async page => {
  page.setDefaultTimeout(10000);
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const primary = () => page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const field = () => page.getByRole("textbox", { name: "議事録ブロック2", exact: true });
  const boot = async () => {
    await page.reload();
    await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await primary().getByRole("button", { name: "議事録を編集", exact: true }).click();
    await field().waitFor(); await field().click();
  };
  const count = () => page.evaluate(() => window.__mon.calls.filter(call => call.cmd === "save_minutes_draft").length);
  await boot();
  const during = await field().evaluate(element => {
    const range = document.createRange(); range.selectNodeContents(element); range.collapse(false);
    const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
    const before = element.innerHTML;
    element.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "て" }));
    const results = [];
    for (const inputType of ["insertParagraph", "insertLineBreak", "insertCompositionText"]) {
      for (const isComposing of [true, false]) {
        const event = new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType, isComposing, data: "て" });
        results.push({ inputType, isComposing, allowed: element.dispatchEvent(event), unchanged: element.innerHTML === before });
      }
    }
    return results;
  });
  check(during.every(result => result.allowed && result.unchanged),
    `composition beforeinput was intercepted or injected a newline: ${JSON.stringify(during)}`);
  check(await count() === 0, "composition issued a durable save");
  const original = await field().textContent();
  await field().evaluate(element => {
    element.textContent = "点検日を確認する";
    element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertCompositionText", isComposing: true, data: "点検日を確認する" }));
  });
  check(await count() === 0, "intermediate composition was saved");
  const paste = await field().evaluate(element => {
    const before = element.innerHTML, data = new DataTransfer(); data.setData("text/plain", "割込み");
    const allowed = element.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data }));
    return { allowed, unchanged: before === element.innerHTML };
  });
  check(!paste.allowed && paste.unchanged, "paste changed active composition");
  await primary().getByRole("button", { name: "編集を終了", exact: true }).click();
  check(await field().getAttribute("contenteditable") === "true", "composition allowed editing to end");
  await page.waitForTimeout(1000);
  check(await count() === 0, "composition debounce saved unconfirmed text");
  await field().evaluate(element => element.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "点検日を確認する" })));
  await page.waitForFunction(() => window.__mon.drafts.a.payload.blocks[1].content.map(node => node.text ?? "").join("") === "点検日を確認する", null, { timeout: 10000 });
  check(await field().textContent() === "点検日を確認する", "compositionend lost confirmed Japanese text");
  check(await count() === 1, "compositionend did not save exactly one confirmed update");
  await field().click(); await field().press("End"); await field().press("Enter");
  await page.waitForFunction(() => window.__mon.drafts.a.payload.blocks[1].content.map(node => node.text ?? "").join("") === "点検日を確認する\n", null, { timeout: 10000 });
  check(await count() === 2, "plain Enter after composition stopped adding one newline");
  const nativeFlag = await field().evaluate(element => {
    const before = element.innerHTML;
    const allowed = element.dispatchEvent(new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType: "insertParagraph", isComposing: true }));
    return { allowed, unchanged: before === element.innerHTML };
  });
  check(nativeFlag.allowed && nativeFlag.unchanged, "native composing flag without compositionstart injected a newline");

  await boot();
  const chip = page.locator('.minutes-rich-field [data-evidence-id]').first();
  await chip.waitFor();
  const references = () => page.getByRole("complementary", { name: "項目の固定根拠一覧", exact: true });
  for (const options of [{ active: true }, { active: true, click: true }, { isComposing: true }, { keyCode: 229 }, { repeat: true }]) {
    const allowed = await chip.evaluate((element, options) => {
      const field = element.closest('.minutes-rich-field');
      if (options.active) field.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
      if (options.click) { element.click(); return true; }
      return element.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", ...options }));
    }, options);
    await page.waitForTimeout(100);
    check(allowed && !await references().isVisible(), `IME or held Enter opened evidence: ${JSON.stringify(options)}`);
    if (options.active) await chip.evaluate(element => element.closest('.minutes-rich-field').dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  }
  check(await count() === 0, "unchanged evidence composition saved a document");
  await chip.focus(); await chip.press("Enter");
  await references().waitFor();
  await page.getByRole("button", { name: "根拠一覧を閉じる", exact: true }).click();
  await references().waitFor({ state: "hidden" });
  return { passed: 6, original, results: ["composition-owned beforeinput retains native conversion and caret", "only confirmed Japanese text saves once", "plain Enter after composition adds exactly one newline", "composition blocks paste, edit exit and debounce save", "native composing flag guards without compositionstart", "IME/229/repeat do not open evidence; plain Enter still does"] };
}
