async (page) => {
  const url = "http://127.0.0.1:1420/";
  await page.unroute(url);
  await page.route(url, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/audio-deletion-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  await page.goto(url);
  await page.waitForFunction(() => !!window.audioDeletionFixture);
  return "Full main window uses simulated audio-deletion IPC only";
}
