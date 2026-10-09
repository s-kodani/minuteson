async page => {
  const url = "http://127.0.0.1:1439/";
  await page.unroute(url);
  await page.route(url, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,
      '<script type="module">await import("/tests/partial-recording-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  await page.goto(url); await page.waitForFunction(() => !!window.partialRecordingFixture);
  return "Synthetic partial-recording IPC in full main window; no native capture";
}
