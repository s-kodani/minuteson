// Synthetic admission delay and timeout. No native deletion is performed.
async (page,capturePath) => {
  const check=(value,message)=>{if(!value)throw Error(message);};
  await page.evaluate(()=>{
    window.dataFixture.blocker='録音通知履歴の確認・更新中';
    window.audioDeletionFixture.admissionHold=true;
    window.audioDeletionFixture.admissionTimeout=true;
  });
  const row=page.locator('.meeting-table tbody tr').filter({has:page.getByRole('button',{name:'既存会議',exact:true})}).first();
  await row.locator('.meeting-row-actions > button').click();
  const remove=page.getByRole('menuitem',{name:'既存会議の音声だけを削除',exact:true});
  await remove.waitFor();
  await page.waitForFunction(()=>!document.querySelector('[role="menuitem"][aria-label="既存会議の音声だけを削除"]').disabled);
  check(await remove.isEnabled(),'history polling disabled audio deletion');
  await remove.click();
  await page.waitForFunction(()=>typeof window.audioDeletionFixture.releaseAdmission==='function');
  await page.getByText('通知履歴の処理完了を待っています',{exact:true}).waitFor();
  await page.screenshot({path:capturePath('notice-deletion-wait.png')});
  await page.evaluate(()=>window.audioDeletionFixture.releaseAdmission());
  await page.getByText('通知履歴の処理が終わらないため、削除を開始できませんでした',{exact:false}).first().waitFor();
  check(await page.getByRole('button',{name:'同じ音声削除操作の結果を確認',exact:true}).count()===0,'known timeout left an unknown request');
  await page.evaluate(()=>{window.audioDeletionFixture.admissionHold=false;window.audioDeletionFixture.admissionTimeout=false;});
  await row.locator('.meeting-row-actions > button').click();
  await remove.waitFor();await remove.click();
  await page.getByRole('heading',{name:'音声だけの削除を確認',exact:true}).waitFor();
  check(await page.evaluate(()=>!window.dataFixture.calls.some(c=>c.command==='start_data_operation')),'review deleted data without consent');
  await page.screenshot({path:capturePath('notice-deletion-retry.png')});
  return {passed:5,results:['history polling does not disable review','native wait status is visible','timeout leaves no unknown request','timeout can retry','retry still requires explicit consent']};
}
