import { test } from "node:test";
import assert from "node:assert/strict";
import { createModalOrder, modalKeyGuard, outsideDialog } from "../src/modal-state.ts";
import { readFileSync } from "node:fs";

test("only actual backdrop coordinates count as outside, not content or padding", () => {
  const bounds = { left: 24, top: 32, right: 504, bottom: 432 };
  for (const [x, y] of [[24, 32], [504, 432], [24, 200], [300, 32], [300, 200]]) assert.equal(outsideDialog(x, y, bounds), false);
  for (const [x, y] of [[23, 200], [505, 200], [300, 31], [300, 433]]) assert.equal(outsideDialog(x, y, bounds), true);
});

const key = (overrides: Partial<Parameters<typeof modalKeyGuard>[0]> = {}) => ({ key: "Escape", defaultPrevented: false, isComposing: false, keyCode: 27, repeat: false, ...overrides });

test("modal keyboard handling leaves conversion and previously handled keys alone", () => {
  assert.equal(modalKeyGuard(key({ isComposing: true })), "ignore");
  assert.equal(modalKeyGuard(key({ keyCode: 229 })), "ignore");
  assert.equal(modalKeyGuard(key({ defaultPrevented: true })), "ignore");
  assert.equal(modalKeyGuard(key({ isComposing: true, repeat: true })), "ignore");
});

test("modal keyboard handling consumes only repeated Escape, not normal cancellation or Tab repetition", () => {
  assert.equal(modalKeyGuard(key({ repeat: true })), "consume");
  assert.equal(modalKeyGuard(key()), "handle");
  assert.equal(modalKeyGuard(key({ key: "Tab", repeat: true })), "handle");
  assert.equal(modalKeyGuard(key({ key: "Enter" })), "handle");
});

test("all common modal key routes use the same conversion and Escape-repeat guard", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source, /const guard = modalKeyGuard\(event\)/);
  assert.match(source, /guard === "consume"\) \{ event\.preventDefault\(\); event\.stopPropagation\(\); return; \}/);
  assert.doesNotMatch(source, /externalContent && (?:event\.repeat|\(event\.isComposing)/);
  assert.match(source, /element\.addEventListener\("keydown",handle\)/);
  assert.match(source, /externalContent \|\| event\.target instanceof Element && event\.target\.closest\("\.toast-host"\)\) keyDown\(event\)/);
  assert.match(source, /isComposing:event\.nativeEvent\.isComposing/);
});

test("native dialogs use showModal modality, not a permanent ARIA modal declaration", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source, /element\.showModal\(\)/);
  assert.doesNotMatch(source, /aria-modal=/);
  assert.match(source, /element\.close\(\)/);
});

test("reopening a modal changes native order without moving hosts or replacing retained inputs", () => {
  const order = createModalOrder<object>();
  const first = { input: { value: "retained", selectionStart: 3 } }, second = {};
  const hosts = [first, second];
  const input = first.input;
  order.opened(first); order.closed(first); order.opened(second); order.opened(first);
  assert.equal(order.top(hosts), first);
  assert.deepEqual(hosts, [first, second]);
  assert.equal(first.input, input);
  assert.deepEqual(input, { value: "retained", selectionStart: 3 });
  order.closed(first);
  assert.equal(order.top([second]), second);
});

test("queue admission opens the native top layer without reparenting a stable portal host", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  const state = readFileSync(new URL("../src/modal-state.ts", import.meta.url), "utf8");
  const guard = source.indexOf("if (!latest.current.open || !element.isConnected || element.open || top && top !== parentModal.current) return;");
  const opening = source.indexOf("element.showModal();");
  assert.ok(guard >= 0 && opening > guard);
  assert.ok(source.indexOf("modalOrder.opened(element);") > opening);
  assert.doesNotMatch(source + state, /prepareModalHost|\.append(?:Child)?\(/);
});

test("removed nested openers restore inside the surviving parent modal, never its inert background", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source, /const boundary = parentModal.current \?\? panel/);
  assert.match(source, /boundary\?\.querySelector<HTMLElement>\("h2, h3"\)/);
  assert.match(source, /!remaining \|\| remaining.contains\(target\)/);
  assert.match(source, /\[hidden\], \[inert\], dialog:not\(\[open\]\)/);
  assert.match(source, /target === fallback.current && target.tabIndex < 0/);
  assert.match(source, /if \(document.activeElement === target\) restorePending.current = false/);
  assert.match(source, /target !== document.body && target !== document.documentElement/);
});

test("confirmation cancellation does not inherit a settings dialog's custom close label", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source, /const label = owned \? owned.phase === "failed" \? "閉じる" : "キャンセル" : replacement\?\.dismissLabel \?\? dismissLabel/);
  assert.match(source, /const customDismissText = owned \? undefined : replacement \? replacement.dismissText : dismissText/);
  assert.match(source, /aria-label=\{customDismissText \?\? `確認を\$\{label\}`\}/);
});

test("replacement reviews preserve mounted settings and share native confirmation ownership", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source,/ref=\{baseBody\} className="common-modal-body" hidden=\{!!owned \|\| !!replacement\}>\{children\}/);
  assert.match(source,/className="common-modal-replacement" hidden=\{!replacement \|\| !!owned\}/);
  assert.match(source,/aria-label=\{owned \|\| replacement \? undefined : attributes\["aria-label"\]\}/);
  assert.match(source,/replacement\?\.labelledBy \?\? attributes\["aria-labelledby"\]/);
  assert.match(source,/latest.current.replacement \|\| latest.current.blocked/);
  assert.match(source,/confirmations.getSnapshot\(\)\?\.parent === element \|\| topOpenModal\(\) !== element/);
  assert.match(source,/baseBody.current\?\.querySelector<HTMLElement>\("h2, h3"\)/);
  assert.match(source,/if \(document.activeElement === destination\) replacedView.current = null/);
  assert.match(source,/suspended.current = false;\s*element.scrollTop = saved.scrollTop/);
});

test("a read-busy replacement exposes safe cancellation after readiness without stealing a chosen field", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source,/replacementInitialFocus.current = !usable && !cancel/);
  assert.match(source,/replacementInitialFocus.current && !effectiveBlocked && !owned && topOpenModal\(\) === element/);
  assert.match(source,/document.activeElement === heading \|\| document.activeElement === element \|\| document.activeElement === document.body/);
  assert.match(source,/replacementInitialFocus.current = false/);
});

test("confirmation phase transitions recover only lost focus in the actual front dialog", () => {
  const source = readFileSync(new URL("../src/common-modal.tsx", import.meta.url), "utf8");
  assert.match(source, /const displayedConfirmation = owned \?\? \(confirmation && !confirmation.parent && attributes\["aria-labelledby"\] === `\$\{confirmation.id\}-title`/);
  assert.match(source, /if \(!displayedConfirmation \|\| !element\?\.open \|\| topOpenModal\(\) !== element\) return/);
  assert.match(source, /active === document.body \|\| active === element \|\| active === heading/);
  assert.match(source, /element.contains\(active\) && active.matches\(":disabled"\)/);
  assert.match(source, /if \(!lost\) return/);
  assert.match(source, /\(cancel \?\? heading\)\?\.focus\(\{ preventScroll: true \}\)/);
  assert.match(source, /\[displayedConfirmation\?\.id, displayedConfirmation\?\.phase, open, ref\]/);
});

test("modal top follows native opening order, not portaled trigger DOM order", () => {
  const order = createModalOrder<object>();
  const outer = {}, inner = {}, third = {};
  order.opened(outer); order.opened(inner);
  assert.equal(order.top([inner, outer]), inner);
  order.opened(third);
  assert.equal(order.top([third, inner, outer]), third);
  order.closed(third);
  assert.equal(order.top([inner, outer]), inner);
  order.closed(inner);
  assert.equal(order.top([outer]), outer);
  order.opened(inner);
  assert.equal(order.top([inner, outer]), inner);
  assert.equal(order.top([]), undefined);
});

test("unowned native dialogs conservatively block the modal queue", () => {
  const order = createModalOrder<object>();
  const owned = {}, unowned = {};
  order.opened(owned);
  assert.equal(order.top([unowned, owned]), unowned);
  assert.equal(order.top([owned, unowned]), unowned);
});

test("nested confirmations and info popovers share the actual top-layer owner", () => {
  for (const name of ["common-modal.tsx", "confirmation-service.ts", "info-popover.tsx"]) {
    const source = readFileSync(new URL(`../src/${name}`, import.meta.url), "utf8");
    assert.match(source, /topOpenModal/);
    assert.doesNotMatch(source, /document\.querySelector(?:<[^>]+>)?\("dialog[^"\n]*\[open\]/);
  }
  const service = readFileSync(new URL("../src/confirmation-service.ts", import.meta.url), "utf8");
  assert.match(service, /modal && !modal.hasAttribute\("data-common-modal"\)/);
  assert.match(service, /const parent = modal \?\? null/);
});
