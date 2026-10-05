async (page) => {
  const url = "http://127.0.0.1:1420/";
  await page.unroute(url);
  await page.route(url, async route => {
    const response = await route.fetch();
    const html = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,
      '<script type="module">await import("/tests/deletion-browser-fixture.js");</script>');
    await route.fulfill({ response, body: html });
  });
  await page.goto(url);
  await page.waitForFunction(() => !!window.deletionFixture);
  return "deletion simulated IPC ready; no real audio or files changed";
}
