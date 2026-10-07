export interface PublicationReview {
  meetingId: string; kind: "analysis" | "minutes"; id: string; contentHash: string;
  revisionNo: number; meetingLockVersion: number; currentId: string | null; restoring: boolean;
}
export interface PublicationReceipt { id: string; contentHash: string; previousId: string | null; meetingLockVersion: number }
export function assertPublicationReceipt(review: PublicationReview, value: unknown): asserts value is PublicationReceipt {
  const result = value as PublicationReceipt | null;
  if (!result || result.id !== review.id || result.contentHash !== review.contentHash || result.previousId !== review.currentId
    || !Number.isSafeInteger(review.meetingLockVersion) || review.meetingLockVersion < 0
    || review.meetingLockVersion >= Number.MAX_SAFE_INTEGER || result.meetingLockVersion !== review.meetingLockVersion + 1) {
    throw Error("採用・復元の保存結果を照合できません。確認した版を保持して結果だけを再確認してください。");
  }
}
export function publicationFromHead(review: PublicationReview, value: unknown): PublicationReceipt {
  const state = value as { currentId?: string; previousId?: string | null; meetingLockVersion?: number; versions?: { id: string; contentHash: string; publicationState: string }[] } | null;
  const version = Array.isArray(state?.versions) ? state.versions.filter(v => v.id === review.id) : [];
  if (!state || state.currentId !== review.id || version.length !== 1 || version[0].publicationState !== "committed") {
    throw Error("採用・復元の完了をまだ確認できません。新しい変更命令は送らず、確認した版を保持しています。");
  }
  const receipt = { id: state.currentId, contentHash: version[0].contentHash, previousId: state.previousId, meetingLockVersion: state.meetingLockVersion };
  assertPublicationReceipt(review, receipt);
  return receipt;
}
export const publicationEvent = (review: PublicationReview) => JSON.stringify([review.meetingId, review.kind, review.id, review.meetingLockVersion + 1]);
export function samePublicationRequest(prior: { review: PublicationReview; command: string; argsKey: string }, review: PublicationReview, command: string, args: unknown) {
  return JSON.stringify(prior.review) === JSON.stringify(review) && prior.command === command && prior.argsKey === JSON.stringify(args);
}

/** Historical stale documents are state, not a new event. A false-to-true
 * authoritative transition is notified once; restoring old history stays quiet. */
export class SourceUpdateTracker {
  private seen = new Map<string, boolean>();
  private serial = 0;
  observe(id: string, contentHash: string, updated: boolean): string | null {
    const key = JSON.stringify([id, contentHash]), before = this.seen.get(key);
    this.seen.set(key, updated);
    return before === false && updated ? JSON.stringify([key, ++this.serial]) : null;
  }
}
