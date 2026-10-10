async page => {
  const results = [], check = (condition, message) => { if (!condition) throw Error(message); };
  const count = command => page.evaluate(command => window.modelDownloadFixture.calls.filter(call => call.command === command).length, command);
  const boot = async () => { await page.goto(`http://127.0.0.1:1420/?download-case=${Math.random()}`); await page.getByRole("heading", { name: "モデル取得の合成検証", exact: true }).waitFor(); await page.getByText("共有準備: 待機", { exact: true }).waitFor(); };
  const input = () => page.getByRole("textbox", { name: "取得定義（JSON）", exact: true });
  const preview = async () => { await input().fill(await page.evaluate(() => window.modelDownloadFixture.definition)); await page.getByRole("button", { name: "取得定義を確認", exact: true }).click(); await page.getByRole("dialog", { name: "モデルの取得内容を確認", exact: true }).waitFor(); };
  const dialog = () => page.locator("dialog[open]");
  const consent = async () => { await dialog().getByRole("checkbox").check(); };
  const start = async () => { await preview(); await consent(); await dialog().getByRole("button", { name: "確認した定義で取得する", exact: true }).click(); };
  const unknown = () => page.getByLabel("未確定のモデル取得操作", { exact: true });
  const recover = async () => { await page.getByRole("button", { name: "同じ操作の停止・結果確認", exact: true }).click(); };
  const verified = () => page.getByLabel("照合済み取得素材", { exact: true });
  const copyReview = async () => { await page.getByRole("button", { name: "照合済み素材のコピーを確認", exact: true }).click(); await page.getByRole("dialog", { name: "管理領域へのコピーを確認", exact: true }).waitFor(); };
  const copy = async () => { await copyReview(); await consent(); await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).click(); };
  await boot(); await preview();
  check(await count("start_model_download") === 0 && await dialog().getByRole("button", { name: "確認した定義で取得する", exact: true }).isDisabled(), "preview started or omitted acknowledgement");
  const warnings = await dialog().innerText(); check(warnings.includes("真正性は未検証") && warnings.includes("未実行・未適用") && warnings.includes("https://example.com") && warnings.includes("33 bytes"), "review details missing");
  check(await page.evaluate(() => document.activeElement?.hasAttribute("data-dialog-cancel")), "review did not initially focus cancel");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  check(await count("start_model_download") === 0 && await page.getByRole("button", { name: "取得定義を確認", exact: true }).evaluate(e => e === document.activeElement), "escape wrote or did not restore opener");
  results.push("review memory-only, bounded definition/source/license/size/SHA warnings, explicit checkbox, cancellation focus and Escape without writes");

  await boot(); await start(); await verified().waitFor();
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 0 && await count("check_model_compatibility") === 0, "download copied or ran automatically");
  await copyReview(); check(await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).isDisabled(), "copy reused download consent");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" }); check(await count("place_downloaded_model") === 0, "cancelled copy wrote");
  await copy(); await dialog().waitFor({ state: "hidden" });
  check(await count("place_downloaded_model") === 1 && await page.evaluate(() => window.modelDownloadFixture.changed === 1 && Array.from(window.modelDownloadFixture.placements.values()).every(value => !value.ready)), "explicit copy did not refresh or claimed ready");
  results.push("one explicit download and separate fresh copy consent; no CPU execution or application; placement callback never grants ready");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "forgedReview");
  await input().fill(await page.evaluate(() => window.modelDownloadFixture.definition)); await page.getByRole("button", { name: "取得定義を確認", exact: true }).click();
  await page.getByLabel("モデル取得・コピーの要対応状態", { exact: true }).waitFor();
  check(await count("start_model_download") === 0 && await dialog().count() === 0, "forged review accepted");
  results.push("review recipe hash is independently bound to original definition before enabling download");

  for (const mode of ["lost", "forgedReceipt"]) {
    await boot(); await page.evaluate(mode => window.modelDownloadFixture.mode = mode, mode); await start(); await unknown().waitFor();
    check(await verified().count() === 0 && await count("start_model_download") === 1 && await page.getByRole("button", { name: "取得定義を確認", exact: true }).isDisabled(), "unknown/forged receipt released writer");
    await recover(); await verified().waitFor();
    check(await count("start_model_download") === 1 && await count("cancel_model_inspection") === 1 && await count("read_model_download") === 1 && await unknown().count() === 0, "readback restarted or left wrong operation");
    const bound = await page.evaluate(() => { const calls = window.modelDownloadFixture.calls, start = calls.find(c => c.command === "start_model_download"), cancel = calls.find(c => c.command === "cancel_model_inspection"), read = calls.find(c => c.command === "read_model_download"); return cancel.args.token === start.args.token && read.args.downloadId === start.args.token && read.args.token !== start.args.token && read.args.expectedRecipeHash === start.args.expectedRecipeHash; });
    check(bound, "cancel/read used wrong identity"); results.push(`${mode}: retain exact writer, cancel first, wait for original termination, read-only fresh-token recovery without retry`);
  }

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "earlyLost"); await start(); await unknown().waitFor(); await recover();
  await page.getByText("共有準備: 操作中", { exact: true }).waitFor();
  check(await count("read_model_download") === 0 && await count("start_model_download") === 1 && await unknown().count() === 1, "active writer recovery proceeded before termination");
  await page.evaluate(() => { try { window.modelDownloadFixture.finish(); } catch {} }); await recover(); await unknown().waitFor({ state: "hidden" });
  check(await verified().count() === 0 && await count("read_model_download") === 1 && (await page.getByLabel("モデル取得結果", { exact: true }).innerText()).includes("未完了"), "cancelled partial claimed success");
  results.push("lost IPC while native writer remains active cannot release lease, read or retry; actual terminal incomplete is preserved");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "delay"); await start();
  await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1);
  const token = await page.evaluate(() => Array.from(window.modelDownloadFixture.active)[0]);
  await page.evaluate(async token => { await window.modelDownloadFixture.progress(token, 25); await window.modelDownloadFixture.progress(token, 2); await window.modelDownloadFixture.progress(token, 33, true); }, token);
  await page.waitForFunction(() => document.querySelector('[aria-label="モデル取得進捗"]')?.textContent.includes("25 / 33"));
  check((await page.getByLabel("モデル取得進捗", { exact: true }).innerText()).includes("25 / 33"), "progress regressed or wrong hash accepted");
  await page.evaluate(async token => window.modelDownloadFixture.progress(token, 33), token);
  await page.waitForFunction(() => document.querySelector('[aria-label="モデル取得進捗"]')?.textContent.includes("33 / 33"));
  check(await verified().count() === 0 && (await page.getByLabel("モデル取得進捗", { exact: true }).innerText()).includes("完了は未確認"), "100% progress granted verified success");
  await page.keyboard.press("Escape"); check(await dialog().isVisible(), "in-flight modal closed on Escape");
  await page.evaluate(() => window.modelDownloadFixture.failCancel = true); await dialog().getByRole("button", { name: "停止を要求する", exact: true }).click();
  await dialog().getByLabel("モデル取得・コピーの要対応状態", { exact: true }).waitFor();
  check(await dialog().isVisible() && await count("start_model_download") === 1, "failed cancel released operation");
  await page.evaluate(() => window.modelDownloadFixture.failCancel = false); await dialog().getByRole("button", { name: "停止を要求する", exact: true }).click();
  check(await dialog().isVisible(), "cancel acknowledgement treated as terminal");
  await page.evaluate(() => window.modelDownloadFixture.finish()); await dialog().waitFor({ state: "hidden" }); await recover(); await unknown().waitFor({ state: "hidden" });
  results.push("bounded monotonic operation/hash-bound progress never proves completion; cancel failure and acknowledgement keep in-flight dialog blocked until owned worker terminates");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "lost"); await start(); await unknown().waitFor();
  const original = await page.evaluate(() => window.modelDownloadFixture.calls.find(call => call.command === "start_model_download").args.token);
  await page.getByRole("button", { name: "設定の表示切替", exact: true }).click(); await page.getByRole("heading", { name: "モデルの初回取得（macOS）", exact: true }).waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "設定の表示切替", exact: true }).click(); await unknown().waitFor();
  check((await unknown().innerText()).includes(original) && await count("start_model_download") === 1, "settings navigation lost original writer");
  await recover(); await verified().waitFor(); results.push("settings unmount/remount retains uncertain writer identity and cancels only that writer; recovery never substitutes new write UUID");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.seed()); await page.getByRole("button", { name: "取得記録を再読込", exact: true }).click();
  await page.getByText("完了記録あり（実体未照合）", { exact: false }).waitFor(); check(await verified().count() === 0, "cold marker granted verified state");
  await page.getByRole("button", { name: "この取得の実体を照合", exact: true }).click(); await verified().waitFor();
  check(await count("start_model_download") === 0 && await count("place_downloaded_model") === 0 && await count("read_model_download") === 1, "cold readback wrote or auto copied");
  results.push("restart-equivalent native marker listing remains unverified until explicit physical readback; no automatic resume/copy");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "lost"); await start(); await unknown().waitFor();
  await page.evaluate(() => window.modelDownloadFixture.forgedReadback = true); await recover(); await page.getByRole("button", { name: "結果未確定のまま確認を終了（ファイル保全）", exact: true }).waitFor();
  check(await unknown().count() === 1 && await verified().count() === 0 && await count("start_model_download") === 1, "forged physical recovery released writer");
  await page.evaluate(() => window.modelDownloadFixture.forgedReadback = false); await recover(); await verified().waitFor();
  results.push("forged readback never clears original uncertainty or enables copy; corrected same-operation readback is required");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "lost"); await start(); await unknown().waitFor();
  await page.evaluate(() => window.modelDownloadFixture.readMissing = true); await recover(); await page.getByRole("button", { name: "結果未確定のまま確認を終了（ファイル保全）", exact: true }).waitFor();
  await page.getByRole("button", { name: "結果未確定のまま確認を終了（ファイル保全）", exact: true }).click(); await unknown().waitFor({ state: "hidden" });
  check(await count("start_model_download") === 1 && await verified().count() === 0 && await page.evaluate(() => window.modelDownloadFixture.operations.size === 1), "retiring unknown deleted or replayed");
  results.push("explicit retirement rechecks cancellation tombstone, actual termination and native listing; preserves unknown files and never reports success");

  await boot(); await start(); await verified().waitFor(); await page.evaluate(() => window.modelDownloadFixture.mode = "lost"); await copy(); await unknown().waitFor(); await recover(); await unknown().waitFor({ state: "hidden" });
  check(await count("place_downloaded_model") === 1 && await count("get_downloaded_model_placement_result") === 1 && await page.evaluate(() => window.modelDownloadFixture.changed === 1) && (await page.getByLabel("モデル取得結果", { exact: true }).innerText()).includes("現在の本体"), "lost copy replayed or confused historical receipt with availability");
  results.push("lost copy is resolved via exact download+recipe+review+package-bound historical receipt, not repeated or treated as current physical readiness");

  await boot(); await start(); await verified().waitFor(); await page.evaluate(() => window.modelDownloadFixture.mode = "delay"); await copy();
  await page.waitForFunction(() => window.modelDownloadFixture.active.size === 1); await dialog().getByRole("button", { name: "停止を要求する", exact: true }).click(); await page.evaluate(() => window.modelDownloadFixture.finish());
  await unknown().waitFor(); await recover(); await unknown().waitFor({ state: "hidden" });
  check(await count("place_downloaded_model") === 1 && await page.evaluate(() => Array.from(window.modelDownloadFixture.placements.values()).every(p => !p.placementRecorded && !p.ready)), "partial copy claimed complete");
  results.push("cancelled managed copy retains partial intent and never reports placement recorded or replays copy");

  await boot(); await preview(); await page.evaluate(() => window.modelDownloadFixture.setLocked(true)); await dialog().waitFor({ state: "hidden" });
  check(await count("start_model_download") === 0 && await page.getByRole("button", { name: "取得定義を確認", exact: true }).isDisabled(), "recording/operation lock did not invalidate idle consent");
  results.push("recording/operation lock invalidates pending consent without starting download");

  await boot(); await start(); await verified().waitFor(); await copyReview(); await consent();
  await page.evaluate(() => window.modelDownloadFixture.setLocked(true)); await dialog().waitFor({ state: "hidden" });
  await page.evaluate(() => window.modelDownloadFixture.setLocked(false)); await copyReview();
  check((await dialog().innerText()).includes("合成取得素材") && await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).isDisabled() && await count("place_downloaded_model") === 0, "lock discarded only half of receipt/review or preserved copy consent");
  await consent(); await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).click(); await dialog().waitFor({ state: "hidden" });
  check(await count("place_downloaded_model") === 1, "fresh copy consent after lock cannot proceed");
  results.push("recording/operation lock closes copy consent, preserves complete verified metadata pair and requires new acknowledgement after unlock");

  await boot(); await page.evaluate(() => { window.modelDownloadFixture.setParent(true); window.modelDownloadFixture.mode = "earlyLost"; });
  await page.getByRole("heading", { name: "AIモデル", exact: true }).waitFor();
  await page.getByRole("button", { name: "互換性を確認", exact: true }).waitFor(); await start(); await unknown().waitFor();
  check(await page.getByRole("button", { name: "互換性を確認", exact: true }).isDisabled() && await page.getByRole("button", { name: "ファイルを事前検査", exact: true }).isDisabled() && await page.getByRole("button", { name: "同じ操作の停止・結果確認", exact: true }).isEnabled(), "child uncertainty did not lock parent preparation or self-locked recovery");
  await recover(); check(await count("read_model_download") === 0 && await page.getByRole("button", { name: "互換性を確認", exact: true }).isDisabled(), "parent enabled while native writer remains active");
  await page.evaluate(() => { try { window.modelDownloadFixture.finish(); } catch {} }); await recover(); await unknown().waitFor({ state: "hidden" });
  await page.waitForFunction(() => { const button = Array.from(document.querySelectorAll('[aria-label="モデル配置記録"] button')).find(button => button.textContent === "互換性を確認"); return button && !button.disabled; });
  check(await count("start_model_download") === 1 && await count("check_model_compatibility") === 0, "recovery replayed writer or auto executed parent CPU check");
  results.push("production parent locks manual/CPU preparation throughout uncertain child writer, keeps same-operation recovery usable, and unlocks only after actual terminal readback");

  await boot(); await page.evaluate(() => window.modelDownloadFixture.failCatalog = true); await page.getByRole("button", { name: "取得記録を再読込", exact: true }).click();
  await page.getByLabel("取得記録の読込の要対応状態", { exact: true }).waitFor();
  const closes = page.getByRole("button", { name: "エラー通知を閉じる", exact: true }); if (await closes.count()) await closes.last().click();
  check(await page.getByLabel("取得記録の読込の要対応状態", { exact: true }).isVisible(), "toast dismissal erased retained failure");
  results.push("native catalog read failure remains retained after transient toast is dismissed");

  await boot();
  await page.getByLabel("取得定義のJSONファイル（64KiB以内）", { exact: true }).setInputFiles({ name: "oversized.json", mimeType: "application/json", buffer: Buffer.alloc(65_537, 0x78) });
  await page.getByLabel("モデル取得・コピーの要対応状態", { exact: true }).waitFor();
  check(await input().inputValue() === "" && await count("review_model_download") === 0 && await count("start_model_download") === 0, "oversized file read/reviewed or downloaded");
  results.push("oversized definition file is rejected before text parsing, native review or network start");

  await boot(); await page.setViewportSize({ width: 390, height: 720 }); await preview();
  check(await dialog().evaluate(element => { const r = element.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth && r.top >= 0 && r.bottom <= innerHeight && element.scrollHeight > element.clientHeight; }), "small viewport dialog overflows or cannot scroll");
  await page.screenshot({ path: "download-mobile-review.png" }); await page.keyboard.press("Escape");
  results.push("small-screen source/SHA review remains within viewport and scrollable with cancel/focus semantics");
  return { passed: results.length, results };
}
