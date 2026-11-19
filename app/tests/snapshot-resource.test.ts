import test from "node:test";
import assert from "node:assert/strict";
import { SnapshotResource, observeInvalidations, observeSnapshots } from "../src/snapshot-resource.ts";

const deferred = <T>() => { let resolve!: (value: T) => void, reject!: (cause: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
function fixture() {
  type Value = { revision: number };
  const reads: ReturnType<typeof deferred<Value>>[] = [];
  let event!: (value: Value) => void, poll!: () => void, focus!: () => void, stopped = 0;
  const resource = new SnapshotResource<Value>({
    read: () => { const value = deferred<Value>(); reads.push(value); return value.promise; },
    subscribe: async accept => { event = accept; return () => { stopped++; }; },
    accept: (current, value) => current && current.revision > value.revision ? current : value,
    intervalMs: 1000,
    schedule: read => { poll = read; return () => { stopped++; }; },
    focus: read => { focus = read; return () => { stopped++; }; },
  });
  return { resource, reads, event: (value: Value) => event(value), poll: () => poll(), focus: () => focus(), stopped: () => stopped };
}
test("snapshot readers share one subscription and coalesce focus and timer reads", async () => {
  const f = fixture(); const a = f.resource.subscribe(() => {}), b = f.resource.subscribe(() => {});
  await flush(); f.poll(); f.focus(); assert.equal(f.reads.length, 1);
  f.reads[0].resolve({ revision: 1 }); await flush();
  a(); assert.equal(f.stopped(), 0); b(); assert.equal(f.stopped(), 3);
});
test("a failed older read cannot overwrite a successful newer event", async () => {
  const f = fixture(), stop = f.resource.subscribe(() => {}); await flush();
  f.event({ revision: 3 }); f.reads[0].reject(Error("old failure")); await flush();
  assert.deepEqual(f.resource.getSnapshot(), { snapshot: { revision: 3 }, connected: true, error: "" }); stop();
});
test("an older successful poll cannot roll back a newer event", async () => {
  const f = fixture(), stop = f.resource.subscribe(() => {}); await flush();
  f.event({ revision: 4 }); f.reads[0].resolve({ revision: 2 }); await flush();
  assert.equal(f.resource.getSnapshot().snapshot?.revision, 4); stop();
});
test("unmount invalidates pending reads and a remount verifies retained state", async () => {
  const f = fixture(), stop = f.resource.subscribe(() => {}); await flush(); stop();
  const next = f.resource.subscribe(() => {}); await flush(); assert.equal(f.reads.length, 2);
  f.reads[0].resolve({ revision: 9 }); await flush(); assert.equal(f.resource.getSnapshot().snapshot, null);
  f.reads[1].resolve({ revision: 2 }); await flush(); assert.equal(f.resource.getSnapshot().snapshot?.revision, 2); next();
});

test("read-only invalidations queue one verification after an event or focus during a read", async () => {
  const reads: ReturnType<typeof deferred<void>>[] = [];
  let notify!: () => void, focus!: () => void, stopped = 0;
  const errors: unknown[] = [];
  const resource = observeInvalidations({
    read: () => { const next = deferred<void>(); reads.push(next); return next.promise; },
    subscribe: async next => { notify = next; return () => { stopped++; }; },
    intervalMs: 3000, schedule: () => () => { stopped++; },
    focus: next => { focus = next; return () => { stopped++; }; }, error: error => errors.push(error),
  });
  await resource.ready;
  notify(); focus(); notify(); assert.equal(reads.length, 1);
  reads[0].resolve(); await flush(); assert.equal(reads.length, 2);
  reads[1].resolve(); await flush();
  const last = resource.refresh(); resource.stop(); assert.equal(stopped, 3);
  assert.equal(reads.length, 3); reads[2].reject(Error("late unmounted failure")); await last;
  assert.deepEqual(errors, []);
});

function eventFixture() {
  type Value = { revision: number };
  const reads: ReturnType<typeof deferred<Value>>[] = [], subscriptions: ReturnType<typeof deferred<() => void>>[] = [];
  const timers = new Set<() => void>(), errors: string[] = [];
  let event!: (value: Value) => void, focus!: () => void, current: Value | null = null, stopped = 0;
  const accept = (value: Value) => {
    if (!Number.isSafeInteger(value.revision) || value.revision < 0) throw Error("invalid event");
    if (!current || value.revision > current.revision) current = value;
  };
  const resource = observeSnapshots<Value>({
    read: () => { const value = deferred<Value>(); reads.push(value); return value.promise; },
    subscribe: next => { event = next; const value = deferred<() => void>(); subscriptions.push(value); return value.promise; },
    accept, error: (cause, source) => { errors.push(`${source}: ${String(cause)}`); },
    intervalMs: 500, recoveryOnly: true,
    schedule: next => { timers.add(next); return () => { timers.delete(next); }; },
    focus: next => { focus = next; return () => { stopped++; }; },
  });
  return { resource, reads, subscriptions, timers, errors, current: () => current,
    event: (value: Value) => event(value), focus: () => focus(), stopped: () => stopped,
    subscribed: (index = 0) => subscriptions[index].resolve(() => { stopped++; }),
  };
}
test("healthy event readers subscribe before reading and never schedule normal polling", async () => {
  const f = eventFixture(); assert.equal(f.reads.length, 0);
  f.subscribed(); await f.resource.ready; assert.equal(f.reads.length, 1);
  f.reads[0].resolve({ revision: 1 }); await flush();
  f.event({ revision: 2 }); assert.equal(f.current()?.revision, 2); assert.equal(f.timers.size, 0);
  f.focus(); f.focus(); assert.equal(f.reads.length, 2);
  f.reads[1].resolve({ revision: 2 }); await flush();
  const manual = f.resource.refresh(); assert.equal(f.reads.length, 3);
  f.reads[2].resolve({ revision: 3 }); await manual;
  assert.equal(f.timers.size, 0); f.resource.stop(); assert.equal(f.stopped(), 2);
});
test("failed event subscriptions poll until focus resubscribes and verifies a fresh read", async () => {
  const f = eventFixture(); f.subscriptions[0].reject(Error("offline")); await f.resource.ready;
  assert.equal(f.timers.size, 1); f.reads[0].resolve({ revision: 1 }); await flush();
  assert.equal(f.timers.size, 1);
  [...f.timers][0](); assert.equal(f.reads.length, 2);
  f.focus(); assert.equal(f.subscriptions.length, 2); f.subscribed(1); await flush();
  f.reads[1].resolve({ revision: 2 }); await flush();
  // A read begun before the new subscription cannot finish its verification.
  assert.equal(f.reads.length, 3); assert.equal(f.timers.size, 1);
  f.reads[2].resolve({ revision: 2 }); await flush();
  assert.equal(f.timers.size, 0); assert.equal(f.subscriptions.length, 2); f.resource.stop();
});
test("failed reads and invalid events use recovery polling until a valid subscribed read", async () => {
  const f = eventFixture(); f.subscribed(); await f.resource.ready;
  f.reads[0].reject(Error("read failed")); await flush(); assert.equal(f.timers.size, 1);
  [...f.timers][0](); f.reads[1].resolve({ revision: 1 }); await flush(); assert.equal(f.timers.size, 0);
  f.event({ revision: -1 }); assert.equal(f.timers.size, 1); assert.equal(f.reads.length, 3);
  f.reads[2].resolve({ revision: 2 }); await flush(); assert.equal(f.timers.size, 0);
  assert.deepEqual(f.errors, ["read: Error: read failed", "event: Error: invalid event"]); f.resource.stop();
});
test("a newer event protects against an old failed read and old successful replies", async () => {
  const f = eventFixture(); f.subscribed(); await f.resource.ready;
  f.event({ revision: 4 }); f.reads[0].reject(Error("old failure")); await flush();
  assert.deepEqual(f.errors, []); assert.equal(f.timers.size, 0);
  const read = f.resource.refresh(); f.event({ revision: 6 }); f.reads[1].resolve({ revision: 5 }); await read;
  assert.equal(f.current()?.revision, 6); assert.equal(f.timers.size, 0); f.resource.stop();
});
test("a read begun before invalid event validation cannot publish before recovery verifies again", async () => {
  const f = eventFixture(); f.subscribed(); await f.resource.ready;
  f.event({ revision: -1 }); assert.equal(f.timers.size, 1);
  f.reads[0].resolve({ revision: 1 }); await flush();
  assert.equal(f.current(), null); assert.equal(f.reads.length, 2); assert.equal(f.timers.size, 1);
  f.reads[1].resolve({ revision: 2 }); await flush();
  assert.equal(f.current()?.revision, 2); assert.equal(f.timers.size, 0); f.resource.stop();
});
test("a command reply protects its newer evidence against an in-flight read failure", async () => {
  const f = eventFixture(); f.subscribed(); await f.resource.ready;
  f.resource.accept({ revision: 4 }); f.reads[0].reject(Error("read before command")); await flush();
  assert.equal(f.current()?.revision, 4); assert.deepEqual(f.errors, []); assert.equal(f.timers.size, 0);
  f.resource.stop();
});
test("manual reads during registration still verify again after subscribing", async () => {
  const f = eventFixture(); const manual = f.resource.refresh(); assert.equal(f.reads.length, 1);
  f.subscribed(); await f.resource.ready;
  f.reads[0].resolve({ revision: 1 }); await manual; await flush();
  assert.equal(f.current(), null); assert.equal(f.reads.length, 2);
  f.reads[1].resolve({ revision: 2 }); await flush();
  assert.equal(f.current()?.revision, 2); assert.equal(f.timers.size, 0); f.resource.stop();
});
test("event readers release a delayed subscription and ignore failures after unmount", async () => {
  const f = eventFixture(); f.resource.stop(); f.subscribed(); await f.resource.ready;
  assert.equal(f.reads.length, 0); assert.equal(f.stopped(), 2); assert.equal(f.timers.size, 0);
  const g = eventFixture(); g.subscribed(); await g.resource.ready;
  g.resource.stop(); g.reads[0].reject(Error("late failure")); await flush();
  assert.deepEqual(g.errors, []); assert.equal(g.timers.size, 0);
});
