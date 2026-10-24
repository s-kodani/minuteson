// Synthetic storage paths only. Never opens or moves a real directory.
async (page, capturePath) => {
  const check=(value,message)=>{if(!value)throw Error(message);};
  await page.evaluate(()=>{
    const b=String.fromCharCode(92);
    window.dataFixture.meetingRoot=b.repeat(2)+'?'+b+['C:','会議 データ','mon'].join(b);
    window.dataFixture.modelsRoot=b.repeat(2)+'?'+b+['C:','モデル','mon'].join(b);
  });
  await page.getByRole('button',{name:'設定',exact:true}).click();
  const region=page.getByRole('region',{name:'データと保存',exact:true});
  await region.getByText('会議保存先:',{exact:false}).waitFor();
  const text=await region.innerText();
  check(text.includes('C:'+String.fromCharCode(92)+'会議 データ'),'display path missing');
  check(!text.includes(String.fromCharCode(92).repeat(2)+'?'),'extended path visible');
  check(await page.evaluate(()=>window.dataFixture.meetingRoot.startsWith(String.fromCharCode(92).repeat(2)+'?')),'internal path mutated');
  await region.locator('.data-path').first().scrollIntoViewIfNeeded();
  await page.screenshot({path:capturePath('native-path-display.png')});
  return {passed:3,results:['normal Windows path display','no extended prefix in storage card','original native path preserved']};
}
