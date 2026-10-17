export interface MinutesAdmission {
  meetingId: string;
  meetingTitle: string;
  operationId: string;
  pending: boolean;
  error: string;
}

/** A start request outlives its meeting view. Pending requests are deduplicated;
 * a lost acknowledgement retains the same operation ID for an explicit retry. */
export class MinutesAdmissionStore {
  private operations = new Map<string, string>();
  private active = new Map<string, Promise<void>>();
  private items = new Map<string, MinutesAdmission>();
  private snapshot: MinutesAdmission[] = [];
  private listeners = new Set<() => void>();
  private newId: () => string;
  constructor(newId: () => string = () => crypto.randomUUID()) { this.newId = newId; }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish() {
    this.snapshot = [...this.items.values()];
    this.listeners.forEach(listener => listener());
  }
  observe(meetingId: string, operationIds: string[]) {
    for (const [key, id] of this.operations) if (key.startsWith(`${meetingId}:`) && operationIds.includes(id)) this.operations.delete(key);
    const item = this.items.get(meetingId);
    if (item && !item.pending && operationIds.includes(item.operationId)) { this.items.delete(meetingId); this.publish(); }
  }
  start(meetingId: string, meetingTitle: string, key: string, submit: (operationId: string) => Promise<void>): Promise<void> {
    const pending = this.active.get(meetingId);
    if (pending) return pending;
    const identity = `${meetingId}:${key}`, operationId = this.operations.get(identity) ?? this.newId();
    this.operations.set(identity, operationId);
    this.items.set(meetingId, { meetingId, meetingTitle, operationId, pending: true, error: "" });
    const work = Promise.resolve().then(() => submit(operationId)).then(() => {
      this.operations.delete(identity); this.items.delete(meetingId);
    }, cause => {
      this.items.set(meetingId, { meetingId, meetingTitle, operationId, pending: false, error: String(cause) });
      throw cause;
    }).finally(() => { this.active.delete(meetingId); this.publish(); });
    this.active.set(meetingId, work); this.publish();
    return work;
  }
}

export const minutesAdmissions = new MinutesAdmissionStore();
