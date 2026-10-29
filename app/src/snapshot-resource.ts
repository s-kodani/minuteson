export interface SnapshotResourceState<T> {
  snapshot: T | null;
  connected: boolean;
  error: string;
}

export interface SnapshotTransport<T> {
  read: () => Promise<T>;
  subscribe: (accept: (value: T) => void) => Promise<() => void>;
  accept: (current: T | null, value: T) => T;
  intervalMs: number;
  schedule: (read: () => void, intervalMs: number) => () => void;
  focus: (read: () => void) => () => void;
}

/** One lifecycle per resource key. Events are subscribed before the initial read;
 * delayed failures cannot overwrite newer evidence, and all reads coalesce. */
export class SnapshotResource<T> {
  private state: SnapshotResourceState<T> = { snapshot: null, connected: false, error: "" };
  private listeners = new Set<() => void>();
  private generation = 0;
  private evidence = 0;
  private pending: Promise<void> | null = null;
  private cleanup: (() => void)[] = [];

  private transport: SnapshotTransport<T>;
  constructor(transport: SnapshotTransport<T>) { this.transport = transport; }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    if (this.listeners.size === 1) this.start();
    return () => {
      this.listeners.delete(listener);
      if (!this.listeners.size) this.stop();
    };
  };
  private publish(state: SnapshotResourceState<T>) {
    this.state = state;
    this.listeners.forEach(listener => listener());
  }
  accept = (value: T) => {
    const snapshot = this.transport.accept(this.state.snapshot, value);
    this.evidence++;
    this.publish({ snapshot, connected: true, error: "" });
  };
  refresh = (): Promise<void> => {
    if (this.pending) return this.pending;
    const generation = this.generation, evidence = this.evidence;
    const pending = this.transport.read().then(value => {
      if (generation === this.generation && this.listeners.size) this.accept(value);
    }).catch(cause => {
      if (generation === this.generation && evidence === this.evidence && this.listeners.size) {
        this.publish({ ...this.state, connected: false, error: String(cause) });
      }
    }).finally(() => { if (this.pending === pending) this.pending = null; });
    this.pending = pending;
    return pending;
  };
  private start() {
    const generation = ++this.generation;
    const poll = () => {
      if (generation !== this.generation || !this.listeners.size) return;
      this.cleanup.push(this.transport.schedule(() => { void this.refresh(); }, this.transport.intervalMs));
      this.cleanup.push(this.transport.focus(() => { void this.refresh(); }));
      void this.refresh();
    };
    void this.transport.subscribe(value => {
      if (generation !== this.generation || !this.listeners.size) return;
      try { this.accept(value); }
      catch (cause) { this.publish({ ...this.state, connected: false, error: String(cause) }); }
    }).then(stop => {
      if (generation !== this.generation || !this.listeners.size) { stop(); return; }
      this.cleanup.push(stop);
      poll();
    }).catch(() => {
      // A lost event subscription is recovered by verified polling.
      poll();
    });
  }
  private stop() {
    this.generation++;
    this.pending = null;
    this.cleanup.splice(0).forEach(stop => stop());
    // A new subscriber must verify the retained snapshot before enabling work.
    this.state = { ...this.state, connected: false, error: "" };
  }
}

/** Shared non-React event/poll/focus lifecycle for domain readers with side reads. */
export function observeSnapshots<T>(options: {
  read: () => Promise<unknown>;
  subscribe: (accept: (value: T) => void) => Promise<() => void>;
  accept: (value: T) => void;
  error: (cause: unknown) => void;
  intervalMs: number;
  invalidations?: boolean;
  schedule: SnapshotTransport<T>["schedule"];
  focus: SnapshotTransport<T>["focus"];
}) {
  let disposed = false, pending: Promise<unknown> | null = null, evidence = 0, again = false;
  const stops: (() => void)[] = [];
  const refresh = (): Promise<unknown> => {
    if (disposed) return Promise.resolve();
    if (pending) { if (options.invalidations) again = true; return pending; }
    pending = (async () => {
      do {
        again = false;
        const readEvidence = evidence;
        try { await options.read(); }
        catch (cause) { if (!disposed && evidence === readEvidence) options.error(cause); }
      } while (again && !disposed);
    })().finally(() => {
      pending = null;
      if (again && !disposed) void refresh();
    });
    return pending;
  };
  const poll = () => {
    if (disposed) return;
    stops.push(options.schedule(() => { void refresh(); }, options.intervalMs), options.focus(() => { void refresh(); }));
    void refresh();
  };
  const stopPromise = options.subscribe(value => {
    if (disposed) return;
    if (options.invalidations) { void refresh(); return; }
    try { options.accept(value); evidence++; }
    catch (cause) { options.error(cause); }
  }).then(stop => { if (disposed) stop(); else { stops.push(stop); poll(); } }).catch(cause => {
    if (!disposed) { options.error(cause); poll(); }
  });
  return { refresh, ready: stopPromise, stop: () => { disposed = true; stops.splice(0).forEach(stop => stop()); } };
}

/** Read-only invalidation events share polling, focus, teardown and queued reads.
 * Domain readers own revision checks and effects; no invalidation carries a DTO. */
export function observeInvalidations(options: {
  read: () => Promise<void>;
  subscribe: (notify: () => void) => Promise<() => void>;
  error: (cause: unknown) => void;
  intervalMs: number;
  schedule: SnapshotTransport<void>["schedule"];
  focus: SnapshotTransport<void>["focus"];
}) {
  return observeSnapshots<void>({ ...options, invalidations: true, accept: () => {} });
}
