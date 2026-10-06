import { test } from "node:test";
import assert from "node:assert/strict";
import { infoPosition } from "../src/info-position.ts";

test("info stays near the icon and flips at the right and bottom edges", () => {
  assert.deepEqual(infoPosition({ left: 30, right: 62, top: 40, bottom: 72 }, { left: 0, top: 0, width: 960, height: 720 }, 180), { left: 30, top: 80, width: 320, maxHeight: 632, side: "below" });
  const p = infoPosition({ left: 900, right: 932, top: 670, bottom: 702 }, { left: 0, top: 0, width: 960, height: 720 }, 180);
  assert.equal(p.left, 612); assert.equal(p.top, 482); assert.equal(p.side, "above");
});
test("small viewports bound long content rather than clip it", () => {
  const p = infoPosition({ left: 300, right: 332, top: 400, bottom: 432 }, { left: 0, top: 0, width: 345, height: 480 }, 900);
  assert.equal(p.width, 320); assert.equal(p.maxHeight, 384); assert.equal(p.top, 8); assert.equal(p.left, 12);
  assert.equal(infoPosition({ left: 20, right: 52, top: 40, bottom: 72 }, { left: 0, top: 0, width: 240, height: 400 }, 80).width, 224);
});
test("visual viewport offsets, zoom and classic scrollbar widths are respected", () => {
  const p = infoPosition({ left: 630, right: 662, top: 530, bottom: 562 }, { left: 100, top: 200, width: 585, height: 400 }, 180);
  assert.ok(p.left >= 108 && p.left + p.width <= 677);
  assert.ok(p.top >= 208 && p.top + Math.min(180, p.maxHeight) <= 592);
  assert.equal(p.side, "above");
});
test("every tested edge and long-content placement remains inside the visible bounds", () => {
  for (const width of [240, 345, 960]) for (const height of [180, 640, 900]) for (const x of [8, width / 2, width - 40]) for (const y of [8, height / 2, height - 40]) for (const contentHeight of [80, 180, 2000]) {
    const p = infoPosition({ left: x, right: x + 32, top: y, bottom: y + 32 }, { left: 0, top: 0, width, height }, contentHeight);
    assert.ok(p.left >= 8 && p.left + p.width <= width - 8);
    assert.ok(p.top >= 8 && p.top + Math.min(contentHeight, p.maxHeight) <= height - 8);
  }
});
