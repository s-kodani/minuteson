import test from "node:test";
import assert from "node:assert/strict";
import { SnapshotResource, observeInvalidations } from "../src/snapshot-resource.ts";

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
