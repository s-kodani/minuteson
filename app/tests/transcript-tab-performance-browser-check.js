async (page, _capturePath, {primaryModifier}) => {
  const results = [], measurements = [];
  const preparationTimeout = 30_000;
  const check = (value, message) => { if (!value) throw Error(message); };
  const cdp = page.context().browser()?.browserType().name() === "chromium" ? await page.context().newCDPSession(page) : null;
  await cdp?.send("Performance.enable");
  for (const {count, forced = false, virtual = true} of [
    {count:200, virtual:false}, {count:200, forced:true}, {count:5000}, {count:20000},
  ]) {
    if (measurements.length) await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(({count, forced}) => {
      if (forced) window.monTranscriptVirtualizeThreshold = 0;
      window.monBrowserFixture.setUtterances(Array.from({length:count}, (_, i) => {
        const text = `発言${i}：${"会議の内容を確認するための合成発言です。".repeat(1 + i % 5)}`;
        return {
          id:`performance-${i}`, trackId:"microphone", startUs:i*4_000_000, endUs:i*4_000_000+3_000_000,
          recognizedText:text, effectiveText:text, attributionState:"assigned", speakerIds:[i%2 ? "speaker-a" : "speaker-b"],
        };
      }));
    }, {count, forced});
    await page.getByRole("button", {name:"会議A",exact:true}).click({timeout:preparationTimeout});
    await page.locator('[data-utterance-id="performance-0"]').waitFor({timeout:preparationTimeout});
    const measurement = await page.evaluate(async () => {
      const list = document.querySelector(".utterance-list");
      const rows = () => [...list.querySelectorAll(".utterance-row")];
      const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const median = values => {
        const sorted = [...values].sort((a,b) => a-b);
        return (sorted[(sorted.length-1)>>1]+sorted[sorted.length>>1])/2;
      };
      await paint();
      const original = rows(), samples = [];
      // 遷移完了を待ち、ウォームアップ後のサンプルを記録する。
      for (let i=0; i<10; i++) {
        const tab = document.getElementById(`meeting-tab-${i%2 ? "transcript" : "minutes"}`);
        let mutations = 0;
        const observer = new MutationObserver(records => { mutations += records.length; });
        observer.observe(list, {subtree:true, attributes:true, childList:true, characterData:true});
        const start = performance.now();
        tab.click();
        do { await paint(); } while (tab.getAttribute("aria-selected") !== "true" || tab.disabled);
        observer.disconnect();
        if (i>=2) samples.push({ms:performance.now()-start, mutations});
      }
      const nodesRetained = rows().length === original.length && rows().every((row,i) => row === original[i]);
      let maxMounted = original.length;
      const scrollSamples = [];
      for (const fraction of [0.25, 0.75, 0.5, 1, 0, 0.25, 0.75, 0.5, 1, 0]) {
        const start = performance.now();
        list.scrollTop = fraction * (list.scrollHeight - list.clientHeight);
        await paint();
        const mounted = rows();
        const box = list.getBoundingClientRect();
        if (!mounted.some(row => {
          const bounds = row.getBoundingClientRect();
          return bounds.bottom > box.top && bounds.top < box.bottom;
        })) throw Error(`スクロール後の表示範囲に発言がありません: ${fraction}`);
        scrollSamples.push(performance.now()-start);
        maxMounted = Math.max(maxMounted, mounted.length);
      }
      return {mounted: original.length, maxMounted, nodesRetained, samples, scrollSamples,
        tabMedianMs:median(samples.map(sample=>sample.ms)), scrollMedianMs:median(scrollSamples)};
    });
    await cdp?.send("HeapProfiler.collectGarbage");
    const heapBytes = (await cdp?.send("Performance.getMetrics"))?.metrics.find(metric => metric.name === "JSHeapUsedSize")?.value;
    measurements.push({count, forced, ...measurement, heapBytes});
    console.log("transcript-tab-performance", JSON.stringify(measurements.at(-1)));
    check(virtual ? measurement.maxMounted <= 60 : measurement.mounted === count, `${count} 行のマウント数が想定外です: ${measurement.maxMounted}`);
    check(measurement.nodesRetained, `${count} 行のタブ切替で行が再マウントされました`);
    check(measurement.samples.every(sample => sample.mutations <= 4), `${count} 行のタブ切替で行の DOM が更新されました`);
    results.push(`${count} 行（強制: ${forced}）のタブ切替とスクロール`);
    if (forced) {
      const list = page.getByRole("region", {name:"発言本文", exact:true});
      const row = index => list.locator(`[data-utterance-id="performance-${index}"]`);
      const selectedText = await list.evaluate(list => {
        const bodies = list.querySelectorAll(".utterance-text");
        const range = document.createRange(), selection = window.getSelection();
        range.setStartBefore(bodies[0]);
        range.setEndAfter(bodies[bodies.length - 1]);
        selection.removeAllRanges();
        selection.addRange(range);
        const text = selection.toString();
        selection.removeAllRanges();
        return text;
      });
      check(selectedText.includes("発言0：") && selectedText.includes("発言1：") && !selectedText.includes("発言199："),
        "範囲選択はマウント済みの複数行の本文を取得します");
      await page.evaluate(() => {
        Object.defineProperty(navigator, "clipboard", {configurable:true, value:{write:async ([item]) => {
          window.monBrowserFixture.copiedText = await (await item.getType("text/plain")).text();
        }}});
      });
      await page.getByRole("button", {name:"文字起こしをコピー", exact:true}).click();
      await page.waitForFunction(() => window.monBrowserFixture.copiedText?.includes("発言199："));
      results.push("範囲選択は描画済みの本文を含み、全体コピーは範囲外の発言も含む");

      const focusRow = async (index, action = false) => {
        const target = action ? row(index).locator(".options-modal-trigger") : row(index);
        await target.evaluate(element => element.focus({preventScroll:true}));
      };
      const focused = (index, action = false) => page.waitForFunction(({index, action}) => {
        const row = document.activeElement?.closest(".utterance-row");
        return row?.dataset.utteranceId === `performance-${index}`
          && (action ? document.activeElement.matches(".options-modal-trigger") : document.activeElement === row);
      }, {index, action});
      const scrollFarFrom = async (retained, missing) => {
        await list.evaluate(list => { list.scrollTop = list.scrollHeight; });
        await page.waitForFunction(({retained, missing}) =>
          !!document.querySelector(`[data-utterance-id="performance-${retained}"]`)
          && !document.querySelector(`[data-utterance-id="performance-${missing}"]`)
          && !!document.querySelector('[data-utterance-id="performance-199"]'), {retained, missing});
      };
      await focusRow(2, true);
      await scrollFarFrom(2, 3);
      await focused(2, true);
      await page.keyboard.press("Tab");
      await focused(3);
      check(await row(3).evaluate(row => row.parentElement.getAttribute("aria-posinset")==="4"),
        "4 番目の発言の位置が読み上げ用の情報にありません");
      check(await row(3).evaluate(row => row.parentElement.getAttribute("aria-setsize")==="200"),
        "全 200 発言の件数が読み上げ用の情報にありません");
      await scrollFarFrom(3, 2);
      await page.keyboard.press("Shift+Tab");
      await focused(2, true);
      await focusRow(2);
      await scrollFarFrom(2, 3);
      await page.keyboard.press("ArrowDown");
      await focused(3);
      await scrollFarFrom(3, 2);
      await page.keyboard.press("ArrowUp");
      await focused(2);
      results.push("Tab・Shift+Tab・上下矢印で範囲外の行を描画してフォーカスを移す");

      await row(2).locator(".transcript-body-edit").press("Enter");
      const draft = row(2).locator("textarea");
      await draft.fill("絞り込み中に保持する未保存の本文");
      await scrollFarFrom(2, 3);
      const search = page.getByRole("searchbox", {name:"発言を検索", exact:true});
      await draft.press(`${primaryModifier}+f`);
      await search.fill("合成発言のどれにも一致しない検索語");
      check(await draft.inputValue()==="絞り込み中に保持する未保存の本文", "0 件に絞り込むと編集中の入力を失います");
      await search.fill("");
      await draft.press("Escape");
      await draft.waitFor({state:"detached"});
      results.push("画面外や一致する発言が 0 件のときも未保存の本文を保持する");
    }
  }
  await cdp?.detach();
  return {passed:results.length, results, measurements};
}
