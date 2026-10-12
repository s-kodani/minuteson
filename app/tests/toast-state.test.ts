import { test } from "node:test";
import assert from "node:assert/strict";
import { ToastController, toastKey, type ToastInput } from "../src/toast-state.ts";

function setup() {
  let time = 0, serial = 0;
  const jobs = new Map<number, { at: number; run: () => void }>();
  const c = new ToastController({ now: () => time, start: (run, delay) => { const id = ++serial; jobs.set(id, { at: time + delay, run }); return id; }, stop: h => { jobs.delete(h as number); } });
  const advance = (duration: number) => {
    const end = time + duration;
    for (;;) { const next = [...jobs].sort((a, b) => a[1].at - b[1].at).find(([, j]) => j.at <= end); if (!next) break; time = next[1].at; jobs.delete(next[0]); next[1].run(); }
    time = end;
  };
  return { c, advance, jobs };
}
const notice = (key = "one", props: Partial<ToastInput> = {}): ToastInput => ({ key, level: "success", message: "保存しました", ...props });

test("verified asynchronous recovery clears only its own matching problem", () => {
  const { c } = setup();
  c.show(notice("shared", { level: "error", message: "old publication failed" }));
  c.resolveMatching("shared", "old publication failed");
  assert.equal(c.getSnapshot().length, 0);
  assert.equal(c.getHistorySnapshot().length, 1);
  c.show(notice("shared", { level: "error", message: "new action failed" }));
  const identity = c.getSnapshot()[0].id;
  c.resolveMatching("shared", "old publication failed");
  assert.equal(c.getSnapshot()[0].id, identity);
});

test("ordinary notices expire after four visible seconds and acknowledge only their own event", () => {
  const { c, advance } = setup(); let closed = 0;
  c.show(notice("one", { onClose: () => closed++ })); advance(3999); assert.equal(c.getSnapshot().length, 1);
  advance(1); assert.equal(c.getSnapshot().length, 0); assert.equal(closed, 1);
  c.show(notice()); assert.equal(c.getSnapshot().length, 0);
  c.resolve("one"); c.show(notice()); assert.equal(c.getSnapshot().length, 1);
});

test("a compact one-card host retains queued identities and remaining duration", () => {
  const {c,advance}=setup(); c.show(notice("normal")); advance(1000);
  c.show(notice("error",{level:"error"})); const before=c.getSnapshot().map(t=>[t.key,t.id]);
  c.setVisibleLimit(1); assert.equal(c.getSnapshot().length,1); assert.equal(c.getSnapshot()[0].key,"error"); assert.equal(c.pendingCount,1);
  advance(60000); c.dismiss("error",c.getSnapshot()[0].id);
  assert.deepEqual([c.getSnapshot()[0].key,c.getSnapshot()[0].id],before.find(([key])=>key==="normal"));
  advance(2999); assert.equal(c.getSnapshot().length,1); advance(1); assert.equal(c.getSnapshot().length,0);
});

test("display capacity changes do not acknowledge notices or allow invalid limits", () => {
  const {c}=setup(); let acknowledged=0;
  for(const key of ["a","b","c"])c.show(notice(key,{level:"error",onClose:()=>acknowledged++}));
  c.setVisibleLimit(1); const current=c.getSnapshot();
  for(const limit of [0,4,1.5,NaN]) assert.throws(()=>c.setVisibleLimit(limit));
  assert.equal(c.getSnapshot(),current); c.setVisibleLimit(3);
  assert.equal(c.getSnapshot().length,3); assert.equal(c.pendingCount,0); assert.equal(acknowledged,0);
});
test("meeting/type keys are unambiguous and separate simultaneous problems", () => {
  assert.notEqual(toastKey("a:b", "c"), toastKey("a", "b:c"));
  const { c } = setup(); c.show(notice(toastKey("a", "save"))); c.show(notice(toastKey("b", "save")));
  assert.equal(c.getSnapshot().length, 2);
});
test("errors and explicitly important warnings never time out", () => {
  const { c, advance, jobs } = setup(); c.show(notice("error", { level: "error" })); c.show(notice("audio", { level: "warning", important: true }));
  advance(60 * 60 * 1000); assert.equal(c.getSnapshot().length, 2); assert.equal(jobs.size, 0);
});
test("repeat polling neither duplicates a notice nor extends its deadline", () => {
  const { c, advance } = setup(); c.show(notice()); const id = c.getSnapshot()[0].id; advance(3000); c.show(notice());
  assert.equal(c.getSnapshot()[0].id, id); advance(1000); assert.equal(c.getSnapshot().length, 0);
});
test("changed content replaces the same key and stale dismissal cannot close the new problem", () => {
  const { c, advance } = setup(); c.show(notice()); const old = c.getSnapshot()[0].id; advance(3000);
  c.show(notice("one", { message: "別の失敗", level: "error" })); const current = c.getSnapshot()[0].id;
  assert.notEqual(old, current); c.dismiss("one", old); advance(10000); assert.equal(c.getSnapshot()[0].id, current);
  c.dismiss("one", current); c.show(notice("one", { message: "別の失敗", level: "error" })); assert.equal(c.getSnapshot().length, 0);
  c.show(notice("one", { message: "新しい失敗", level: "error" })); assert.equal(c.getSnapshot().length, 1);
});
test("hover and keyboard focus independently pause and preserve remaining time", () => {
  const { c, advance } = setup(); c.show(notice()); advance(1000); c.pause("one", "hover", true); c.pause("one", "focus", true);
  advance(20000); c.pause("one", "hover", false); advance(10000); assert.equal(c.getSnapshot().length, 1);
  c.pause("one", "focus", false); advance(2999); assert.equal(c.getSnapshot().length, 1); advance(1); assert.equal(c.getSnapshot().length, 0);
});
test("clipped cards preserve their remaining duration until fully visible", () => {
  const { c, advance } = setup(); c.show(notice()); const id = c.getSnapshot()[0].id;
  advance(1000); c.pause("one", "clipped", true, id); advance(20000);
  assert.equal(c.getSnapshot()[0].id, id);
  c.pause("one", "focus", true); c.pause("one", "clipped", false, id); advance(10000);
  assert.equal(c.getSnapshot().length, 1);
  c.pause("one", "focus", false); advance(2999); assert.equal(c.getSnapshot().length, 1);
  advance(1); assert.equal(c.getSnapshot().length, 0);
});
test("a stale visibility check or cleanup cannot pause or resume a replacement", () => {
  const { c, advance } = setup(); c.show(notice()); const old = c.getSnapshot()[0].id;
  c.show(notice("one", { message:"新しい結果" })); const current = c.getSnapshot()[0].id;
  c.pause("one", "clipped", true, current); c.pause("one", "clipped", false, old); advance(10000);
  assert.equal(c.getSnapshot()[0].id, current);
  c.pause("one", "clipped", false, current); c.pause("one", "clipped", true, old);
  advance(3999); assert.equal(c.getSnapshot().length, 1); advance(1); assert.equal(c.getSnapshot().length, 0);
});
test("hidden document and confirmation modal pause all notifications including new ones", () => {
  const { c, advance } = setup(); c.pauseAll("hidden", true); c.show(notice()); advance(10000);
  c.pauseAll("modal", true); c.pauseAll("hidden", false); advance(10000); assert.equal(c.getSnapshot().length, 1);
  c.pauseAll("modal", false); advance(4000); assert.equal(c.getSnapshot().length, 0);
});
test("only three notices are visible; important problems displace normal notices without losing them", () => {
  const { c, advance } = setup(); for (const key of ["a", "b", "c"]) c.show(notice(key)); advance(1000);
  for (const key of ["e1", "e2", "e3", "e4"]) c.show(notice(key, { level: "error" }));
  assert.equal(c.getSnapshot().length, 3); assert.ok(c.getSnapshot().every(n => n.important)); assert.equal(c.pendingCount, 4);
  advance(10000); c.resolve("e4"); assert.equal(c.getSnapshot()[2].key, "e1");
  for (const key of ["e1", "e2", "e3"]) c.resolve(key);
  advance(2999); assert.equal(c.getSnapshot().length, 3); advance(1); assert.equal(c.getSnapshot().length, 0);
});
test("queued notices receive their full remaining display duration after promotion", () => {
  const { c, advance } = setup(); for (const key of ["e1", "e2", "e3"]) c.show(notice(key, { level: "error" }));
  c.show(notice()); advance(100000); c.resolve("e1"); advance(3999); assert.ok(c.getSnapshot().some(n => n.key === "one"));
  advance(1); assert.ok(!c.getSnapshot().some(n => n.key === "one"));
});
test("hover/focus flags of a displaced card cannot leave it permanently paused", () => {
  const { c, advance } = setup(); c.show(notice()); c.pause("one", "focus", true); c.pause("one", "hover", true);
  for (const key of ["e1", "e2", "e3"]) c.show(notice(key, { level: "error" }));
  c.resolve("e1"); advance(4000); assert.ok(!c.getSnapshot().some(n => n.key === "one"));
});
test("action notices have eight seconds; actions are single-flight, pause expiry and expose busy state", async () => {
  const { c, advance } = setup(); let calls = 0, done!: () => void;
  c.show(notice("one", { action: { kind: "undo", label: "元に戻す", run: () => { calls++; return new Promise<void>(resolve => { done = resolve; }); } } }));
  advance(7000); const id = c.getSnapshot()[0].id; const action = c.act("one", id); await c.act("one", id);
  assert.equal(calls, 1); assert.equal(c.getSnapshot()[0].actionBusy, true); advance(10000); assert.equal(c.getSnapshot().length, 1);
  done(); await action; assert.equal(c.getSnapshot()[0].actionBusy, false); advance(1000); assert.equal(c.getSnapshot().length, 0);
});
test("disabled, stale, dismissed and modal-covered actions cannot run", async () => {
  const { c } = setup(); let calls = 0; const action = { kind: "retry" as const, label: "再試行", run: () => { calls++; } };
  c.show(notice("one", { action: { ...action, disabled: true } })); const id = c.getSnapshot()[0].id; await c.act("one", id);
  c.show(notice("one", { action })); c.pauseAll("modal", true); await c.act("one", id); c.pauseAll("modal", false);
  await c.act("one", id + 1); c.dismiss("one", id); await c.act("one", id); assert.equal(calls, 0);
});
test("late action completion cannot alter replacement notices and rejected actions preserve the original", async () => {
  const { c } = setup(); let fail!: (e: Error) => void;
  c.show(notice("one", { action: { kind: "open", label: "対象を開く", run: () => new Promise<void>((_, reject) => { fail = reject; }) } }));
  const running = c.act("one", c.getSnapshot()[0].id); c.show(notice("one", { level: "error", message: "新しい問題" }));
  fail(Error("cannot open")); await assert.rejects(running, /cannot open/); assert.equal(c.getSnapshot()[0].message, "新しい問題"); assert.equal(c.getSnapshot()[0].actionBusy, undefined);
});
test("resolving a problem removes it without executing close callbacks or any operation", () => {
  const { c } = setup(); let closed = 0, updates = 0; const unsubscribe = c.subscribe(() => updates++);
  c.show(notice("one", { level: "error", onClose: () => closed++ })); const first = c.getSnapshot(); assert.equal(c.getSnapshot(), first);
  c.resolve("one"); assert.equal(closed, 0); assert.equal(updates, 2); unsubscribe(); c.show(notice()); assert.equal(updates, 2);
});
test("distinct operation IDs notify identical result text without duplicating repeated reads of one operation", () => {
  const { c, advance } = setup(); c.show(notice("one", { eventId: "operation-1" })); const first = c.getSnapshot()[0].id;
  c.dismiss("one", first); c.show(notice("one", { eventId: "operation-1" })); assert.equal(c.getSnapshot().length, 0);
  c.show(notice("one", { eventId: "operation-2" })); assert.ok(c.getSnapshot()[0].id > first); advance(3000);
  c.show(notice("one", { eventId: "operation-2" })); advance(1000); assert.equal(c.getSnapshot().length, 0);
});

test("acknowledged event receipts survive empty reads, remounts and older event replay", () => {
  const { c } = setup(); let closed = 0;
  const first = notice("one", { eventId: "operation-1", onClose: () => closed++ });
  c.show(first); c.dismiss("one", c.getSnapshot()[0].id);
  c.resolve("one"); c.show(first);
  assert.equal(c.getSnapshot().length, 0);
  c.show(notice("one", { eventId: "operation-2" }));
  const newer = c.getSnapshot()[0];
  c.show(first); assert.equal(c.getSnapshot()[0], newer);
  c.dismiss("one", newer.id); c.resolve("one"); c.show(first);
  assert.equal(c.getSnapshot().length, 0); assert.equal(closed, 1);
  c.show(notice("another-meeting", { eventId: "operation-1" }));
  assert.equal(c.getSnapshot().length, 1);
  c.show(notice("one", { eventId: "operation-1", message: "保存結果に問題が見つかりました", level: "error" }));
  assert.equal(c.getSnapshot().length, 2);
});

test("event receipt retention is bounded and never suppresses unshown or distinct events", () => {
  const { c } = setup();
  for (let i = 0; i <= 512; i++) {
    c.show(notice("one", { eventId: `operation-${i}` }));
    c.dismiss("one", c.getSnapshot()[0].id); c.resolve("one");
  }
  c.show(notice("one", { eventId: "operation-1" })); assert.equal(c.getSnapshot().length, 0);
  c.show(notice("one", { eventId: "operation-0" })); assert.equal(c.getSnapshot().length, 1);
  c.resolve("one"); c.show(notice("one", { eventId: "unshown" }));
  c.resolve("one"); c.show(notice("one", { eventId: "unshown" }));
  assert.equal(c.getSnapshot().length, 1);
});

test("render confirmation requires the exact visible card and is emitted once, not by reception or promotion", () => {
  const { c } = setup(); let rendered = 0;
  c.show(notice("one", { onRendered: () => rendered++ })); const id = c.getSnapshot()[0].id;
  assert.equal(rendered, 0);
  for (const key of ["e1", "e2", "e3"]) c.show(notice(key, { level: "error" }));
  c.markRendered("one", id); assert.equal(rendered, 0);
  c.resolve("e1"); assert.equal(rendered, 0);
  c.markRendered("one", id + 1); assert.equal(rendered, 0);
  c.markRendered("one", id); c.markRendered("one", id); assert.equal(rendered, 1);
  c.show(notice("one", { onRendered: () => rendered++ })); c.markRendered("one", id); assert.equal(rendered, 1);
});

test("hidden and modal-covered cards cannot acknowledge rendering; expiry cannot invent it", () => {
  const { c, advance } = setup(); let rendered = 0;
  c.show(notice("one", { onRendered: () => rendered++ })); const id = c.getSnapshot()[0].id;
  c.pauseAll("hidden", true); c.markRendered("one", id);
  c.pauseAll("modal", true); c.pauseAll("hidden", false); c.markRendered("one", id);
  assert.equal(rendered, 0); c.pauseAll("modal", false); advance(4000);
  c.markRendered("one", id); assert.equal(rendered, 0);
});

test("render callbacks use current input and reject a stale replacement identity", () => {
  const { c } = setup(); let old = 0, current = 0;
  c.show(notice("one", { onRendered: () => old++ })); const first = c.getSnapshot()[0].id;
  c.show(notice("one", { onRendered: () => current++ })); c.markRendered("one", first);
  assert.equal(old, 0); assert.equal(current, 1);
  c.show(notice("one", { eventId: "next", onRendered: () => current++ })); const next = c.getSnapshot()[0].id;
  c.markRendered("one", first); assert.equal(current, 1);
  c.markRendered("one", next); assert.equal(current, 2);
});
