async (page) => {
  const url = "http://127.0.0.1:1420/";
  await page.route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module">await import("/tests/theme-browser-fixture.js");</script>');
    await route.fulfill({ response, body });
  });
  await page.goto(url);
  await page.waitForFunction(() => !!window.themeFixture);
  return "Theme uses dedicated simulated settings and synthetic silence only";
}
