// Run via playwright-cli before the browser checks. Never change app/index.html.
async (page) => {
  const url = "http://127.0.0.1:1420/";
  await page.unroute(url);
  await page.route(url, async route => {
    const response = await route.fetch();
    const html = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u,
      '<script type="module">await import("/tests/data-save-browser-fixture.js"); await import("/src/main.tsx");</script>');
    await route.fulfill({ response, body: html });
  });
  await page.goto(url);
  await page.waitForFunction(() => !!window.dataFixture);
  return "mocked IPC ready; no real files or recording changed";
}
