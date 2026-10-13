// Synthetic IPC ordering, not native storage or recording.
async page => {
  const results = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const region = () => page.getByRole("region", { name: "データと保存", exact: true });
  const boot = async () => {
    await page.reload();
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().getByRole("button", { name: "バックアップを作成", exact: true }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll('.data-save button')].some(b => b.textContent === 'バックアップを作成' && !b.disabled));
  };
  const choose = () => region().getByRole("button", { name: "バックアップを作成", exact: true }).click();
  const starts = () => page.evaluate(() => window.dataFixture.calls.filter(c => c.command === 'start_data_operation').length);
  const finish = async () => {
    await page.evaluate(() => { window.dataFixture.blocker = null; });
    await page.waitForFunction(() => window.dataFixture.snapshot.phase === 'completed');
    check(await starts() === 1, 'backup did not publish exactly once after verified admission release');
  };

  await boot();
  await page.evaluate(() => { window.dataFixture.readyBlocker = 'バックアップ準備の終了待ち'; });
  await choose();
  await region().getByText('バックアップ準備の終了待ち。停止・完了後にバックアップ／復元できます。', { exact: true }).waitFor();
  check(await starts() === 0, 'ready event reused availability from before selection lease');
  check(await page.getByRole('alert').count() === 0, 'normal lease handoff caused an error toast');
  await finish();
  results.push('ready notification before lease release waits for current availability without premature admission');

  await boot();
  await page.evaluate(() => {
    window.dataFixture.holdAvailability = true;
    window.dataFixture.readyBlocker = 'バックアップ準備の終了待ち';
    window.dispatchEvent(new Event('focus'));
  });
  await page.waitForFunction(() => !!window.dataFixture.availabilityComplete);
  await choose();
  await page.waitForFunction(() => window.dataFixture.snapshot.phase === 'ready');
  await page.evaluate(() => {
    window.dataFixture.holdAvailability = false;
    window.dataFixture.availabilityComplete();
  });
  await region().getByText('バックアップ準備の終了待ち。停止・完了後にバックアップ／復元できます。', { exact: true }).waitFor();
  check(await starts() === 0, 'delayed availability from an older revision unlocked a newer ready selection');
  check(await page.getByRole('alert').count() === 0, 'stale availability caused an admission failure');
  await finish();
  results.push('delayed availability for old revision cannot admit newer backup selection');

  await boot();
  await page.evaluate(() => { window.dataFixture.readyBlocker = '準備の終了待ち'; });
  await choose();
  await region().getByText('準備の終了待ち。停止・完了後にバックアップ／復元できます。', { exact: true }).waitFor();
  const notices = await page.evaluate(() => window.dataFixture.calls.filter(c => c.command === 'get_archived_recording_input_notices').length);
  await page.waitForTimeout(1200); // Cross two normal 500 ms notice polling ticks.
  check(await page.evaluate(() => window.dataFixture.calls.filter(c => c.command === 'get_archived_recording_input_notices').length) === notices,
    'ready backup restarted historical notification writes before publication');
  await finish();
  await page.waitForFunction(n => window.dataFixture.calls.filter(c => c.command === 'get_archived_recording_input_notices').length > n, notices);
  results.push('historical notice polling stays paused across ready-to-publish handoff and resumes after completion');
  return results;
}
