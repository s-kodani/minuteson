// Production shared provider under synthetic IPC; no native recording.
async page=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.addInitScript(()=>Object.defineProperty(navigator,"platform",{configurable:true,get:()=>new URLSearchParams(location.search).get("resident")==="macos"?"MacIntel":"Win32"}));
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const logged=(stage,message)=>page.waitForFunction(({stage,message})=>window.closeFixture.calls.some(c=>c.command==="record_input_diagnostic_log"&&c.args.stage===stage&&(message===null?c.args.message===null:!!c.args.message)),{stage,message});
  try{
    for(const platform of ["windows","macos"]){
      await page.goto(`http://127.0.0.1:1420/?resident=${platform}`);await page.waitForFunction(()=>!!window.closeFixture);
      await page.getByRole("navigation",{name:"メインナビゲーション",exact:true}).waitFor();
      await page.evaluate(()=>{const f=window.closeFixture;f.failJournal=true;return f.recordChanged({phase:"recording",sessionId:crypto.randomUUID()});});
      await logged("live-journal",true);await page.clock.runFor(6200);
      check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"journal read failure notified");
      await page.evaluate(()=>window.closeFixture.failJournal=false);await logged("live-journal",null);
      await page.evaluate(()=>window.closeFixture.journalStatus={state:"failed",requestedRevision:2,committedRevision:1,closed:false});
      await logged("live-journal-notice",true);
      await page.evaluate(()=>window.closeFixture.journalStatus={state:"confirmed",requestedRevision:2,committedRevision:2,closed:false});
      await logged("live-journal-notice",null);
      check(await page.evaluate(()=>window.closeFixture.notificationHistory().length===0),"journal write failure/recovery notified");
      results.push(`${platform}: journal diagnostics and recovery stay logged while automatic polling continues silently`);
    }
    return {passed:results.length,results};
  }finally{await page.context().unrouteAll();}
}
