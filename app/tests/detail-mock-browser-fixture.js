// Existing synthetic IPC fixtures only. No native windows, audio or file writes.
if (new URLSearchParams(location.search).get("view") === "minutes") {
  const { installMinutesFixture } = await import("./minutes-browser-fixture.js");
  await installMinutesFixture({ addInitScript: async install => install(), reload: async () => {} });
  await import("../src/main.tsx");
} else {
  await import("./browser-fixture.js");
}
