// Product main/mini rendering with synthetic IPC; never starts native capture.
async page=>{
  const root=new URL(page.url()).origin,results=[];
  const check=(ok,message)=>{if(!ok)throw Error(message);};
  await page.context().route(url=>url.origin===root&&url.pathname==="/",async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const state=async value=>page.waitForFunction(v=>["microphone","system"].every(k=>document.querySelector(`[data-input-kind="${k}"]`)?.dataset.inputState===v),value);
  const set=async (value,epoch,count)=>page.evaluate(([value,epoch,count])=>{
    const f=window.closeFixture,status={state:value,sourceEpoch:epoch,interruptions:count,reason:value==="connected"?null:"clock_changed"};
    f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};
    f.inputStatus={schemaVersion:4,ownerId:f.recording.sessionId,revision:(f.inputStatus?.revision??0)+1,reportAgeMs:0,microphone:{...status},system:{...status},notices:[],noticeLoss:f.queueState.loss};
  },[value,epoch,count]);
  try{
    await page.goto(root+"/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();await page.locator("#recording-title").waitFor();
    await page.getByRole("button",{name:"録音を開始",exact:true}).click();
    const dialog=page.getByRole("dialog",{name:"選択した会議の録音を開始しますか？",exact:true});
    await dialog.getByRole("button",{name:"録音を開始する",exact:true}).click();await dialog.waitFor({state:"hidden"});
    await page.evaluate(()=>{window.closeFixture.refreshInputStatus=true;window.closeFixture.recordChanged({phase:"recording"});});
    await set("connected",1,0);await state("connected");
    await set("disconnected",1,1);await state("disconnected");
    for(const input of ["microphone","system"]){
      const label=page.locator(`[data-input-kind="${input}"]`);
      check((await label.innerText()).includes("スリープ等により取得時計が変更"),`${input} main lost cause`);
      check(!(await label.innerText()).includes("接続済み"),`${input} main fabricated recovery`);
    }
    results.push("both main inputs retain the explicit clock interruption, not a connected/saved claim");
    for(const [width,theme] of [[1280,"light"],[960,"dark"],[360,"dark"]]){
      await page.setViewportSize({width,height:900});
      await page.getByRole("banner").getByRole("combobox",{name:"表示テーマ",exact:true}).selectOption(theme);
      await state("disconnected");
      check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`main clock labels overflow ${width}`);
      await page.screenshot({path:`output/playwright/mon-clock-control/clock-recovery-main-${width}-${theme}.png`});
      results.push(`both clock labels fit ${width}px ${theme}`);
    }
    await set("connected",1,1);await state("unknown");
    results.push("unchanged source epochs cannot claim a recovered main input");
    await set("connected",2,1);await state("connected");
    results.push("new confirmed epochs restore both main labels");
    await page.goto(root+"/?window=recorder");await page.waitForFunction(()=>!!window.closeFixture);
    await page.setViewportSize({width:360,height:116});
    await page.evaluate(()=>{window.closeFixture.refreshInputStatus=true;window.closeFixture.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID()});});
    await set("disconnected",1,1);await state("disconnected");
    for(const input of ["microphone","system"]){
      const label=page.locator(`[data-input-kind="${input}"]`);
      check((await label.getAttribute("title")).includes("スリープ等により取得時計が変更"),`${input} mini lost full accessible cause`);
      check(!(await label.innerText()).includes("接続済み"),`${input} mini fabricated recovery`);
    }
    check(await page.locator(".mini-shell").evaluate(e=>e.scrollHeight<=e.clientHeight+1&&e.scrollWidth<=e.clientWidth+1),"mini clock interruption overflowed native geometry");
    await page.screenshot({path:"output/playwright/mon-clock-control/clock-recovery-mini-360x116.png"});
    results.push("mini keeps both full causes in titles without overflowing 360x116 or claiming recovery");
    await set("connected",2,1);await state("connected");
    results.push("mini recovers only on confirmed new source epochs");
    return {passed:results.length,results};
  }catch(cause){throw Error(`after ${results.length} cases: ${cause}; body=${await page.locator("body").innerText()}`);}
  finally{await page.context().unrouteAll();}
}
