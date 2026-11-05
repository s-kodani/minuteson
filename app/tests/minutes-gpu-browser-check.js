// Run with playwright-cli run-code; every backend response is synthetic.
async page => {
  await page.route("**/gpu-progress.html", route => route.fulfill({contentType:"text/html",body:'<html lang="ja"><div id="root"></div><script type="module" src="/tests/browser-fixture.js"></script></html>'}));
  await page.goto("http://127.0.0.1:1420/gpu-progress.html");
  await page.getByRole("button",{name:"会議A",exact:true}).waitFor();
  await page.evaluate(async()=>{
    const f=window.monBrowserFixture, now=Date.now();
    const job={id:crypto.randomUUID(),meetingId:"meeting-0",meetingTitle:"会議A",operationId:crypto.randomUUID(),kind:"minutes_generation",state:"running",completedUnits:2,totalUnits:10,errorCode:null,outputAnalysisId:null,pauseReason:null,specHash:"a".repeat(64),inputHash:"b".repeat(64),retryOfJobId:null,attempt:1,createdAt:now,updatedAt:now};
    f.jobs=[job];f.notificationJobs=[job];await f.emitNotification();
    f.gpuEvent={jobId:job.id,specHash:job.specHash,attempt:1,unitIndex:2,invocation:crypto.randomUUID(),invocationOrder:1,startedAtUtcMs:now,sequence:1,phase:"input",backend:"metal",merge:false,elapsedMs:12000};
    await f.emitMinutesRuntime(f.gpuEvent);
  });
  const region=page.getByRole("region",{name:"進行中の処理",exact:true});
  await region.getByText(/3工程目：GPU · 入力を解析中/).waitFor();
  if(await region.getByRole("progressbar").getAttribute("value")!=="20")throw Error("GPU phase fabricated completed progress");
  await region.screenshot({path:"output/playwright/minutes-metal-input.png"});
  await page.evaluate(async()=>{const f=window.monBrowserFixture;await f.emitMinutesRuntime({...f.gpuEvent,sequence:2,phase:"generating",elapsedMs:25000});});
  await region.getByText(/GPU · 本文を生成中/).waitFor();
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.notificationJobs=[{...f.notificationJobs[0],state:"paused",updatedAt:Date.now()}];await f.emitNotification();});
  await region.getByText("議事録生成 · 一時停止 2 / 10",{exact:true}).waitFor();
  if(await region.getByText(/GPU ·/).count())throw Error("paused phase was retained");
  await page.evaluate(async()=>{const f=window.monBrowserFixture;f.notificationJobs=[{...f.notificationJobs[0],state:"running",updatedAt:Date.now()}];await f.emitNotification();await f.emitMinutesRuntime({...f.gpuEvent,invocation:crypto.randomUUID(),invocationOrder:2,sequence:1,backend:"cpu",startedAtUtcMs:Date.now(),elapsedMs:1000});});
  await region.getByText(/CPU · 入力を解析中/).waitFor();
  if(await region.getByRole("progressbar").getAttribute("value")!=="20")throw Error("CPU fallback changed completed progress");
  return {passed:4,checks:["GPU input and generation labels","20% remains based on durable units","pause clears runtime phase","CPU choice label"]};
}
