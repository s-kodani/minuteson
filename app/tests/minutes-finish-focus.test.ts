import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/minutes-editor.tsx",import.meta.url),"utf8");
const finish = source.slice(source.indexOf("const finishEditing ="),source.indexOf("const primaryActions ="));
const retry = source.slice(source.indexOf("const retrySaving ="),source.indexOf("const finishEditing ="));

test("minutes editor and toast retries share the same guarded queue retry without reloading or exiting",()=>{
  assert.match(source,/ref=\{retryToggle\}/);
  assert.match(source,/onClick=\{event => void retryFromEditor\(event.currentTarget\)\}/);
  assert.match(source,/run: retrySaving/);
  assert.match(retry,/failure.current \|\| compositions.current.size/);
  assert.match(retry,/await queue.retry\(\)/);
  assert.doesNotMatch(retry,/invoke\(|\.edit\(|queue\.load\(|setEditing\(/);
});

test("minutes retry focuses the surviving entry only after saving and does not override a later focus or modal",()=>{
  assert.ok(retry.indexOf("actionOpener.focus")<retry.indexOf("await retrySaving()"));
  assert.ok(retry.indexOf("await retrySaving()")<retry.indexOf("requestAnimationFrame"));
  assert.match(retry,/queue.snapshot\(\).error \? retryToggle.current : editToggle.current/);
  assert.match(retry,/mounted.current && target\?\.getClientRects\(\).length && !target.disabled/);
  assert.match(retry,/active === actionOpener \|\| active === document.body/);
  assert.match(retry,/!document.querySelector\("dialog\[open\]"\)/);
});

test("minutes exit captures the actual entry before disabling fields and awaits successful saving",()=>{
  assert.match(source,/onClick=\{event => void finishEditing\(event.currentTarget\)\}/);
  const entry=finish.indexOf("actionOpener.focus({ preventScroll: true });");
  const capture=finish.indexOf("const focused = document.activeElement;");
  const running=finish.indexOf("return run(async () => {");
  const save=finish.indexOf("await queue.flush();");
  const exit=finish.indexOf("setEditing(false); setSelection(null);");
  assert.ok(entry>=0&&capture>entry&&running>capture&&save>running&&exit>save);
  assert.match(finish,/failure.current \|\| compositions.current.size/);
});

test("minutes exit restores only its own lost focus and does not steal a later choice or open modal",()=>{
  assert.match(finish,/focused === document.activeElement \|\| document.activeElement === document.body/);
  assert.match(finish,/editorRoot.current\?\.contains\(focused\) \|\| actionOpener === focused/);
  assert.match(finish,/mounted.current && target\?\.getClientRects\(\).length && !target.disabled/);
  assert.match(finish,/active === focused \|\| active === document.body/);
  assert.match(finish,/!document.querySelector\("dialog\[open\]"\)/);
  assert.match(finish,/target.focus\(\{ preventScroll: true \}\)/);
  assert.doesNotMatch(finish,/invoke\(|\.edit\(|queue\.load\(/);
});
