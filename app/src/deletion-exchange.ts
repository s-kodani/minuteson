import type { DataRequest, DataSnapshot } from "./data-operation-state";
import { errorMessage, operationFailureMessage, operationNotStarted } from "./app-error.ts";

export interface DeletionChoice { id: string; pending: boolean; revision: number }
export interface DeletionExchangeState { rpcBusy: boolean; unknown: boolean; error: string }
/** Shared request ownership and uncertain-response reconciliation. Target-specific
 * consent, playback barriers and receipt effects remain in their domain hooks. */
export class DeletionExchange {
  readonly running = { current: false };
  readonly pending: { current: { request: DataRequest; revision: number } | null } = { current: null };
  readonly choice: { current: DeletionChoice | null } = { current: null };
  private state: DeletionExchangeState = { rpcBusy: false, unknown: false, error: "" };
  private listeners = new Set<() => void>();
  private notStartedCode: string;
  constructor(notStartedCode: string) { this.notStartedCode = notStartedCode; }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(next: Partial<DeletionExchangeState>) { this.state = { ...this.state, ...next }; this.listeners.forEach(listener => listener()); }
  setUnknown = (unknown: boolean) => this.update({ unknown });
  setError = (error: string) => this.update({ error });
  run = async (action: () => Promise<void>) => {
    if (this.running.current) return;
    this.running.current = true; this.update({ rpcBusy: true, error: "" });
    try { await action(); } catch (cause) { this.setError(errorMessage(cause)); }
    finally { this.running.current = false; this.update({ rpcBusy: false }); }
  };
  review = async (choice: DeletionChoice, action: () => Promise<void>, notStarted?: () => void) => {
    this.choice.current = choice;
    try { await action(); this.choice.current = null; this.setUnknown(false); }
    catch (cause) {
      if (operationNotStarted(cause, this.notStartedCode)) {
        this.choice.current = null; this.setUnknown(false); notStarted?.();
        throw Error(operationFailureMessage(cause, this.notStartedCode));
      }
      this.setUnknown(true);
      throw Error(`確認の受付を確定できません。削除は実行せず、同じ確認の状態を読み直してください: ${errorMessage(cause)}`);
    }
  };
  send = async (request: DataRequest, revision: number, action: () => Promise<void>, notStarted?: () => void) => {
    this.pending.current = { request, revision };
    try { await action(); this.pending.current = null; this.setUnknown(false); }
    catch (cause) {
      if (operationNotStarted(cause, this.notStartedCode)) {
        this.pending.current = null; this.setUnknown(false); notStarted?.();
        throw Error(operationFailureMessage(cause, this.notStartedCode));
      }
      this.setUnknown(true);
      throw Error(`受付結果を確定できません。別の削除を始めず、同じ操作の結果を確認してください: ${errorMessage(cause)}`);
    }
  };
  reconcile = (snapshot: DataSnapshot, confirmation: string | undefined, matchesReview: (snapshot: DataSnapshot, choice: DeletionChoice) => boolean): boolean => {
    if (!this.state.unknown) return false;
    const choice = this.choice.current, pending = this.pending.current;
    const reviewKnown = choice && matchesReview(snapshot, choice);
    const requestKnown = pending && snapshot.token === pending.request.token && snapshot.revision > pending.revision
      && (snapshot.busy || ["completed", "failed", "cancelled", "attention"].includes(snapshot.phase)
        || snapshot.phase === "ready" && confirmation !== pending.request.confirmationId);
    if (!reviewKnown && !requestKnown) return false;
    this.pending.current = null; this.choice.current = null; this.setUnknown(false);
    return true;
  };
}
