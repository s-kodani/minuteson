async(page,capturePath)=>{
  const results=[],check=(ok,text)=>{if(!ok)throw Error(text);};
  const nav=page.getByRole("navigation",{name:"メインナビゲーション",exact:true});
  const selectStyle=node=>{
    const style=getComputedStyle(node);
    return Object.fromEntries(["appearance","fontSize","lineHeight","borderRadius","borderTopWidth","borderTopColor","backgroundColor","backgroundImage","backgroundSize","paddingTop","paddingRight","paddingBottom","paddingLeft"].map(key=>[key,style[key]]));
  };
  await nav.getByRole("button",{name:"設定",exact:true}).click();
  const themeSelect=page.getByRole("region",{name:"表示設定",exact:true}).getByRole("combobox",{name:"表示モード",exact:true});
  await themeSelect.waitFor();
  const sharedSelectStyle=await themeSelect.evaluate(selectStyle);
  await nav.getByRole("button",{name:"ジョブ一覧",exact:true}).click();
  await page.locator('#job-active-panel .job-list-empty').waitFor();
  await page.screenshot({path:capturePath('job-list-empty.png')});
  await page.getByRole("tab",{name:"履歴",exact:true}).click();
  const history=page.getByRole("region",{name:"過去のジョブ履歴",exact:true});
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===50);
  const firstSelect=history.getByRole("combobox",{name:"履歴の種類",exact:true});
  check(JSON.stringify(await firstSelect.evaluate(selectStyle))===JSON.stringify(sharedSelectStyle),"history select differs from the existing settings select");
  const viewport=page.viewportSize(),originalTheme=await page.evaluate(()=>document.documentElement.dataset.theme);
  for(const theme of ["light","dark"]){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    for(const width of [1200,760,390]){
      await page.setViewportSize({width,height:900});
      const layout=await history.evaluate(section=>{
        const measure=node=>{
          const s=getComputedStyle(node);
          return {height:node.getBoundingClientRect().height,font:s.fontSize,line:s.lineHeight,radius:s.borderRadius,appearance:s.appearance,image:s.backgroundImage,padding:s.padding,border:s.border,background:s.backgroundColor};
        };
        // A new select with no screen class must inherit the same defaults.
        const added=document.createElement('select');added.innerHTML='<option>新規の選択欄</option>';added.style.position='absolute';added.style.left='-10000px';document.body.append(added);
        try{
          const selects=[...section.querySelectorAll('.job-history-filters select')].map(measure);
          const text=section.querySelector('.job-history-actions>span').getBoundingClientRect(),button=section.querySelector('.job-history-actions>button').getBoundingClientRect();
          return {selects,added:measure(added),centerDifference:Math.abs(text.top+text.height/2-button.top-button.height/2),overflow:document.documentElement.scrollWidth>innerWidth};
        }finally{added.remove();}
      });
      check(layout.selects.every(select=>JSON.stringify(select)===JSON.stringify(layout.added)),`shared select styling missed a new field or history override at ${theme}/${width}`);
      check(layout.added.height===40&&layout.added.font==="13px"&&layout.added.appearance==="none"&&layout.added.image!=="none",`shared select metrics/chevron missing at ${theme}/${width}`);
      check(layout.centerDifference<=1,`retention label is not centered on refresh at ${theme}/${width}`);
      check(!layout.overflow,`history toolbar overflows at ${theme}/${width}`);
      if(width===1200||width===390)await page.screenshot({path:capturePath(`job-history-controls-${theme}-${width}.png`)});
    }
  }
  await page.emulateMedia({forcedColors:"active"});
  check(await firstSelect.evaluate(node=>getComputedStyle(node).appearance==="auto"&&getComputedStyle(node).backgroundImage==="none"),"forced colors did not restore the native select");
  await page.emulateMedia({forcedColors:"none"});
  await page.evaluate(theme=>{if(theme===undefined)delete document.documentElement.dataset.theme;else document.documentElement.dataset.theme=theme;},originalTheme);
  await page.setViewportSize(viewport);
  await firstSelect.focus();await page.keyboard.press('Tab');await page.keyboard.press('Shift+Tab');
  check(await firstSelect.evaluate(node=>getComputedStyle(node).outlineStyle!=="none"),"select keyboard focus is invisible");
  const meetingIcon=nav.getByRole("button",{name:"会議一覧",exact:true}).locator('.nav-glyph svg');
  const jobIcon=nav.getByRole("button",{name:"ジョブ一覧",exact:true}).locator('.nav-glyph svg');
  const iconStyle=node=>({viewBox:node.getAttribute('viewBox'),stroke:node.getAttribute('stroke'),width:node.getAttribute('stroke-width'),cap:node.getAttribute('stroke-linecap'),join:node.getAttribute('stroke-linejoin'),size:[node.getAttribute('width'),node.getAttribute('height')]});
  check(JSON.stringify(await meetingIcon.evaluate(iconStyle))===JSON.stringify(await jobIcon.evaluate(iconStyle)),"meeting list icon differs from the shared navigation icon style");
  results.push("history selects inherit shared defaults, retention text is vertically centered across themes/widths, forced colors and focus remain usable, and meeting navigation uses the shared SVG icon");
  const historyReads=()=>page.evaluate(()=>window.lifecycleFixture.calls.filter(c=>c.command==="list_job_history").length);
  let reads=await historyReads();
  await page.evaluate(()=>window.lifecycleFixture.changed());
  await page.waitForFunction(count=>window.lifecycleFixture.calls.filter(c=>c.command==="list_job_history").length>count,reads);
  await history.getByRole("button",{name:"さらに読み込む",exact:true}).click();
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===57);
  reads=await historyReads();
  await page.evaluate(()=>window.lifecycleFixture.changed());
  await history.getByRole("button",{name:"最新の履歴を表示",exact:true}).waitFor();
  check(await historyReads()===reads,"native update discarded an older history page");
  check(await history.locator('.job-history-rows>li').count()===57,"native update hid loaded older rows");
  await history.getByRole("combobox",{name:"履歴の結果",exact:true}).selectOption("failed");
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===12);
  await history.getByRole("button",{name:/の詳細$/}).first().click();
  const detail=page.getByRole("dialog",{name:"ジョブの詳細",exact:true});await detail.waitFor();
  check((await detail.innerText()).includes("合成の処理失敗"),"failure detail missing");
  await page.keyboard.press("Escape");await detail.waitFor({state:"hidden"});
  await history.getByRole("combobox",{name:"履歴の種類",exact:true}).selectOption("model");
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===0);
  await history.getByText("該当する履歴はありません",{exact:true}).waitFor();
  await history.getByRole("combobox",{name:"履歴の結果",exact:true}).selectOption("");
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===4);
  await page.evaluate(()=>window.lifecycleFixture.readFail=true);
  await history.getByRole("button",{name:"ジョブ履歴を再読込",exact:true}).click();
  await history.getByRole("alert").filter({hasText:"合成の履歴取得失敗"}).waitFor();
  check(await history.locator('.job-history-rows>li').count()===4,"read failure cleared history");
  await page.evaluate(()=>window.lifecycleFixture.readFail=false);
  await history.getByRole("button",{name:"再試行",exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('.job-history [role="alert"]'));
  await history.getByRole("combobox",{name:"履歴の種類",exact:true}).selectOption("");
  await page.waitForFunction(()=>document.querySelectorAll('.job-history-rows>li').length===50);
  await page.screenshot({path:capturePath("job-history.png")});
  results.push("history has filtered paginated persisted results, details, empty state and recoverable read errors");
  await nav.getByRole("button",{name:"ボイス一覧",exact:true}).click();
  const voice=page.getByRole("region",{name:"田中 太郎",exact:true});await voice.waitFor();
  check((await voice.innerText()).includes("人物ID 9b3b937d"),"person identity label missing");
  check((await voice.locator('.voice-person-models').innerText()).includes("3D-Speaker ERes2Net base / モデル名不明"),"person summary lost model identity");
  check(await voice.locator('.voice-sample-model').count()===2,"sample model labels missing");
  await page.screenshot({path:capturePath("voice-model-names.png")});
  for(const width of [900,760]){await page.setViewportSize({width,height:1000});check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"voice model names overflow");}
  results.push("voice library distinguishes person IDs and per-sample registered model names including unknown models");
  await page.evaluate(()=>localStorage.setItem("MON_LIFECYCLE_GUIDANCE","pending"));
  await page.goto(new URL("/?initial=1&guidance-read-failure=1",page.url()).href);
  await page.waitForFunction(()=>window.lifecycleFixture?.calls.some(call=>call.command==="record_background_read_failure"&&call.args.source==="model_download_guidance"));
  check(await page.evaluate(async()=>{
    const {toasts}=await import('/src/toast.tsx');
    return localStorage.getItem("MON_LIFECYCLE_GUIDANCE")==="pending"
      && !toasts.getHistorySnapshot().some(notice=>notice.message.includes("合成の初回案内取得失敗"));
  }),"a passive guidance read failure changed the saved preference or entered notification history");
  results.push("failed startup guidance reads are logged silently and can recover next launch");
  await page.goto(new URL("/?initial=1",page.url()).href);
  const guidance=page.getByRole("dialog",{name:"ローカルAIモデルをダウンロード",exact:true});await guidance.waitFor();
  await page.evaluate(()=>window.lifecycleFixture.failDismiss=true);
  await guidance.getByRole("button",{name:"あとで",exact:true}).click();
  await guidance.getByRole("alert").filter({hasText:"合成の案内終了保存失敗"}).waitFor();
  check(await guidance.isVisible(),"failed dismissal hid guidance");
  await page.evaluate(()=>window.lifecycleFixture.failDismiss=false);
  await guidance.getByRole("button",{name:"モデルの設定を開く",exact:true}).click();
  await guidance.waitFor({state:"hidden"});await page.getByRole("heading",{name:"設定",exact:true}).waitFor();
  check(!await page.evaluate(()=>window.lifecycleFixture.calls.some(c=>c.command==="start_model_download")),"guidance started a download");
  await page.reload();await nav.getByRole("button",{name:"設定",exact:true}).waitFor();
  check(await guidance.count()===0||!await guidance.isVisible(),"dismissed guidance returned after restart");
  await page.evaluate(()=>localStorage.setItem("MON_LIFECYCLE_GUIDANCE","pending"));await page.reload();await guidance.waitFor();
  await page.keyboard.press("Escape");await guidance.waitFor({state:"hidden"});
  check(await page.evaluate(()=>localStorage.getItem("MON_LIFECYCLE_GUIDANCE"))==="done","Escape did not persist dismissal");
  results.push("first launch guidance persists either action and Escape, retries save failure and never downloads automatically");
  return {passed:results.length,results};
}
