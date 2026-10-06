async (page) => {
  const url = "http://127.0.0.1:1420/";
  await page.unrouteAll();
  await page.route(url, async route => {
    const response = await route.fetch();
    const body = (await response.text()).replace(/<script type="module" src="\/src\/main\.tsx[^"]*"><\/script>/u, '<script type="module" src="/tests/common-modal-browser-fixture.tsx"></script>');
    await route.fulfill({ response, body });
  });
  await page.goto(url); await page.waitForFunction(() => !!window.modalFixture);
  return "common modal fixture ready; no native changes";
}
