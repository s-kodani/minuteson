// Native IPC is synthetic. Cleanup status stays backend-only; no actual recording.
async page=>{
  const results=[],check=(v,m)=>{if(!v)throw Error(m);};
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  const boot=async phase=>{
    await page.goto("http://127.0.0.1:1420/");await page.waitForFunction(()=>!!window.closeFixture);
    await page.evaluate(phase=>sessionStorage.setItem("MON_CAPTURE_TEST_RECOVERY_FIXTURE",JSON.stringify({schemaVersion:1,revision:2,phase,runId:null,errorMessage:phase==="attention"?"synthetic owned cleanup pending":null})),phase);
    await page.reload();await page.waitForFunction(()=>!!window.closeFixture);
    await page.waitForFunction(()=>{const b=[...document.querySelectorAll("button")].find(b=>b.textContent.trim().endsWith("新しい録音"));return b&&!b.disabled;});
  };
  try{
    for(const phase of ["checking","attention","clear"]){
      await boot(phase);
      check(await page.getByRole("region",{name:"テスト音声の回復",exact:true}).count()===0,"cleanup panel remains");
      check(await page.getByRole("button",{name:"会議管理・録音・解析操作",exact:true}).count()===0,"landing management dialog remains");
      await page.getByRole("button",{name:"設定",exact:true}).click();await page.getByRole("heading",{name:"設定",exact:true}).waitFor();
      await page.getByRole("button",{name:"前の画面へ戻る",exact:true}).click();await page.getByRole("heading",{name:"会議一覧",exact:true}).waitFor();
      check(await page.evaluate(()=>window.closeFixture.calls.every(c=>!["get_capture_test_recovery","retry_capture_test_recovery","start_capture_test","start_recording"].includes(c.command))),"UI initiated cleanup or recording");
      results.push(`backend ${phase} state has no cleanup UI and does not block settings/back navigation`);
    }
    await boot("attention");
    await page.getByRole("button",{name:"新しい録音",exact:true}).click();
    await page.locator(".capture-test-panel > button").click();
    const consent=page.getByRole("dialog",{name:"5秒の録音前テストを開始しますか？",exact:true});
    await consent.getByRole("button",{name:"5秒テストを開始する",exact:true}).click();
    await consent.locator(".action-error[role=alert]:visible").waitFor();
    await consent.getByRole("button",{name:"確認を閉じる",exact:true}).click();
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="reconcile_capture_test_start"));
    await page.locator(".capture-test-dialog").getByRole("button",{name:"確認を閉じる",exact:true}).click();
    await page.getByRole("button",{name:"設定",exact:true}).click();
    await page.getByRole("heading",{name:"設定",exact:true}).waitFor();
    check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="start_capture_test").length===1&&window.closeFixture.testRetired.length===1),"rejected start was replayed or not fenced");
    check(await page.getByText("開始要求の受付結果が未確認です。開始は再送しません。",{exact:true}).count()===0,"rejected start remained pending");
    results.push("cleanup rejection fences the absent start, restores navigation and does not resend capture");
    await page.evaluate(()=>window.closeFixture.request(true));
    await page.waitForFunction(()=>window.closeFixture.calls.some(c=>c.command==="finish_main_close"));
    check(await page.getByRole("dialog",{name:"未回収のテスト音声を保持して終了しますか？",exact:true}).count()===0,"obsolete exit confirmation remains");
    results.push("retained test audio does not add an exit dialog or force deletion");
    await page.screenshot({path:"background-test-cleanup.png"});
    return {passed:results.length,results};
  }finally{await page.evaluate(()=>sessionStorage.removeItem("MON_CAPTURE_TEST_RECOVERY_FIXTURE"));}
}
