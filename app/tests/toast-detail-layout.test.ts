import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name: string) => readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
test("one notification owner remains in the header through detail navigation", () => {
  const main = source("main.tsx"), toast = source("toast.tsx"), css = source("prototype-alignment.css");
  assert.equal(main.match(/<ToastHost\b/g)?.length, 1);
  assert.match(main, /<ToastHost docked=\{detail\} \/><\/header>/);
  assert.match(main, /observer.observe\(header\)/);
  assert.match(toast, /data-docked=\{docked\}/);
  assert.match(css, /\.main-column \.topbar>\.toast-host\[data-docked=true\] \{ position:static;/);
  assert.match(css, /\.topbar>\.toast-host\[data-docked=true\]:empty \{ display:none;/);
  assert.match(toast, /toasts.pause\(toast.key,"clipped",!visible,toast.id\)/);
  assert.match(toast, /toasts.pause\(toast.key,"clipped",false,toast.id\)/);
});
