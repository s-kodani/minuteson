async page => {
  const { readFile } = await import("node:fs/promises");
  const { resolve } = await import("node:path");
  const html = await readFile(resolve(process.cwd(), "../prototypes/meeting-app-preview.html"), "utf8");
  await page.route("**/prototype-preview", route => route.fulfill({ contentType: "text/html", body: html }));
  await page.goto("http://127.0.0.1:1420/prototype-preview");
  await page.evaluate(() => {
    navigate("minutes");
    document.querySelector("#minutes-paper td").textContent = String.raw`path\name|literal`;
  });
  const downloaded = page.waitForEvent("download");
  await page.getByRole("button", { name: /Markdown出力/ }).click();
  const content = await readFile(await (await downloaded).path(), "utf8");
  if (!content.includes(String.raw`path\\name\|literal`)) throw Error("Markdown cell lost its literal backslash or pipe");
  return { passed: 1, results: ["Markdown download escapes backslashes before table delimiters"] };
}
