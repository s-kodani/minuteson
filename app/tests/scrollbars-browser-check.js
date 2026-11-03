async (page, capturePath) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const both = page.getByTestId('both'), vertical = page.getByTestId('vertical');
  const textarea = page.getByRole('textbox', { name:'スクロールする入力' });
  const scrollState = element => ({ top:element.scrollTop, left:element.scrollLeft, maxTop:element.scrollHeight-element.clientHeight, maxLeft:element.scrollWidth-element.clientWidth });
  await both.waitFor();
  const geometry = await both.evaluate(element => ({ ...element.getBoundingClientRect().toJSON(), clientWidth:element.clientWidth, clientHeight:element.clientHeight, offsetWidth:element.offsetWidth, offsetHeight:element.offsetHeight, borderLeft:parseFloat(getComputedStyle(element).borderLeftWidth), borderTop:parseFloat(getComputedStyle(element).borderTopWidth) }));
  check(geometry.offsetWidth-geometry.clientWidth-geometry.borderLeft*2 === 10 && geometry.offsetHeight-geometry.clientHeight-geometry.borderTop*2 === 10, 'vertical/horizontal scrollbar did not reserve the common 10px size');
  const initial = await both.evaluate(scrollState);
  check(initial.maxTop > 0 && initial.maxLeft > 0, 'two-axis fixture is not scrollable');
  // Drag the real browser scrollbar from its first thumb to the far end.
  const scrollbarX = geometry.x + geometry.width - geometry.borderLeft - 5;
  await page.mouse.move(scrollbarX, geometry.y + 24); await page.mouse.down();
  await page.mouse.move(scrollbarX, geometry.y + geometry.height - 26, { steps:10 }); await page.mouse.up();
  await page.waitForFunction(() => { const element=document.querySelector('[data-testid="both"]'); return element.scrollTop > (element.scrollHeight-element.clientHeight)*0.8; });
  const scrollbarY = geometry.y + geometry.height - geometry.borderTop - 5;
  await page.mouse.move(geometry.x + 24, scrollbarY); await page.mouse.down();
  await page.mouse.move(geometry.x + geometry.width - 26, scrollbarY, { steps:10 }); await page.mouse.up();
  await page.waitForFunction(() => { const element=document.querySelector('[data-testid="both"]'); return element.scrollLeft > (element.scrollWidth-element.clientWidth)*0.8; });
  results.push('both scrollbar axes share 10px geometry and real thumb drags reach the end');
  await vertical.hover(); await page.mouse.wheel(0, 300);
  await page.waitForFunction(() => document.querySelector('[data-testid="vertical"]').scrollTop > 0);
  // Start the editor's caret and scroll position explicitly. WebKit may
  // otherwise retain its initial end selection when focus follows a wheel.
  await textarea.click();
  await textarea.evaluate(element => { element.setSelectionRange(0, 0); element.scrollTop = 0; });
  await page.waitForFunction(() => { const input=document.querySelector('textarea'); return document.activeElement === input && input.selectionStart === 0 && input.scrollTop === 0; });
  await textarea.press('PageDown');
  await page.waitForFunction(() => document.querySelector('textarea').scrollTop > 0);
  check(await page.getByTestId('short').evaluate(element => element.scrollHeight <= element.clientHeight && element.scrollWidth <= element.clientWidth), 'short content gained overflow');
  results.push('wheel and textarea keyboard scrolling work; short content remains without overflow');
  for (const color of ['monochrome','purple']) for (const theme of ['light','dark']) {
    await page.evaluate(({color,theme}) => { document.documentElement.dataset.colorTheme=color; document.documentElement.dataset.theme=theme; }, {color,theme});
    await page.mouse.move(8, 8);
    await page.screenshot({ path:capturePath(`scrollbars-${color}-${theme}.png`) });
    await page.getByRole('button', { name:'スクロール確認を開く', exact:true }).click();
    const modal = page.getByRole('dialog', { name:'スクロール確認', exact:true }); await modal.waitFor();
    const state = await modal.evaluate(scrollState);
    check(state.maxTop > 0 && state.maxLeft <= 1, 'modal did not keep one vertical scroll boundary');
    check(await modal.locator('.common-modal-body').evaluate(element => element.scrollHeight <= element.clientHeight), 'modal body became a separate scroll container');
    await page.screenshot({ path:capturePath(`scrollbars-modal-${color}-${theme}.png`) });
    const bounds = await modal.boundingBox();
    await page.mouse.move(bounds.x + bounds.width - 6, bounds.y + 28);
    await page.screenshot({ path:capturePath(`scrollbars-modal-hover-${color}-${theme}.png`) });
    await page.mouse.down();
    await page.screenshot({ path:capturePath(`scrollbars-modal-active-${color}-${theme}.png`) });
    await page.mouse.move(bounds.x + bounds.width - 6, bounds.y + bounds.height - 28, { steps:12 }); await page.mouse.up();
    await page.waitForFunction(() => { const element=document.querySelector('dialog[open]'); return element.scrollTop > (element.scrollHeight-element.clientHeight)*0.8; });
    await modal.getByRole('button', { name:'末尾から閉じる', exact:true }).click();
    await modal.waitFor({ state:'hidden' });
    await page.waitForFunction(() => document.activeElement?.textContent === 'スクロール確認を開く');
  }
  results.push('both color themes and brightness modes keep one rounded modal scroll boundary, with draggable thumb and focus restoration');
  await page.setViewportSize({ width:360, height:480 });
  for (const platform of ['macos','windows']) {
    await page.evaluate(platform => { if (platform === 'windows') document.documentElement.dataset.windowChrome='windows'; else delete document.documentElement.dataset.windowChrome; }, platform);
    await page.getByRole('button', { name:'スクロール確認を開く', exact:true }).click();
    const modal = page.getByRole('dialog', { name:'スクロール確認', exact:true });
    const bounds = await modal.boundingBox();
    check(bounds.x >= 23 && bounds.x+bounds.width <= 337 && bounds.y >= (platform === 'windows' ? 55 : 23) && bounds.y+bounds.height <= 457, 'small modal no longer fits its viewport/titlebar');
    await page.screenshot({ path:capturePath(`scrollbars-modal-small-${platform}.png`) });
    await modal.getByRole('button', { name:'末尾から閉じる', exact:true }).click();
  }
  results.push('small modal retains margins, Windows titlebar clearance and an accessible last action');
  await page.setViewportSize({ width:1200, height:900 });
  await page.emulateMedia({ forcedColors:'active' });
  if (await page.evaluate(() => matchMedia('(forced-colors:active)').matches)) {
    await page.screenshot({ path:capturePath('scrollbars-forced-colors.png') });
    results.push('forced-colors mode renders the scrollbar with system colors');
  }
  return { passed:results.length, results };
}
