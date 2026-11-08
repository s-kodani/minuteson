// Production UI, synthetic IPC only. Assert bounded DOM work rather than a
// wall-clock threshold: timings are attached for comparison on the same host.
async page => {
  const results = [], measurements = [];
  const check = (value, message) => { if (!value) throw Error(message); };
  for (const count of [347, 1284, 5000]) {
    if (measurements.length) await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(count => window.monBrowserFixture.setUtterances(Array.from({length:count}, (_, i) => ({
      id:`performance-${i}`, trackId:"microphone", startUs:i*4_000_000, endUs:i*4_000_000+3_000_000,
      recognizedText:`発言${i}：会議の内容を確認するための合成発言です。`,
      effectiveText:`発言${i}：会議の内容を確認するための合成発言です。`,
      attributionState:"assigned", speakerIds:[i%2 ? "speaker-a" : "speaker-b"],
    }))), count);
    await page.getByRole("button", {name:"会議A",exact:true}).click();
    await page.waitForFunction(count => document.querySelectorAll(".utterance-row").length === count, count);
    const measurement = await page.evaluate(async () => {
      const list = document.querySelector(".utterance-list");
      const original = [...list.children];
      const samples = [];
      const paint = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      // Warm both panels before timing. Wait for the actual navigation barrier,
      // not just an animation frame that could precede the tab commit.
      for (let i=0; i<10; i++) {
        const tab = document.getElementById(`meeting-tab-${i%2 ? "transcript" : "minutes"}`);
        let mutations = 0;
        const observer = new MutationObserver(records => { mutations += records.length; });
        observer.observe(list, {subtree:true, attributes:true, childList:true, characterData:true});
        const start = performance.now();
        tab.click();
        do { await paint(); } while (tab.getAttribute("aria-selected") !== "true" || tab.disabled);
        observer.disconnect();
        if (i>=2) samples.push({tab:i%2 ? "transcript" : "minutes", ms:performance.now()-start, mutations});
      }
      const sorted = samples.map(sample => sample.ms).sort((a,b) => a-b);
      return {
        count:original.length, samples,
        medianMs:(sorted[3]+sorted[4])/2, maxMs:sorted.at(-1),
        nodesRetained:original.every((row,i) => row === list.children[i]),
      };
    });
    measurements.push(measurement);
    console.log("transcript-tab-performance", JSON.stringify(measurement));
    results.push(`${count} utterances retain row identity and bound tab-switch DOM work`);
  }
  for (const measurement of measurements) {
    check(measurement.nodesRetained, `${measurement.count} utterances were remounted`);
    check(measurement.samples.every(sample => sample.mutations <= 4), `${measurement.count} utterances updated individual rows on tab switch`);
  }
  return {passed:results.length, results, measurements};
}
