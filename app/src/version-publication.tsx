import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { IssueStatus, toasts } from "./toast";
import { toastKey } from "./toast-state";
import { assertPublicationReceipt, publicationEvent, publicationFromHead, samePublicationRequest, type PublicationReview } from "./version-publication-state";

interface Pending { review: PublicationReview; command: string; argsKey: string; error: string; active: boolean }
// A response may be lost while navigating away. Retain the exact expectation
// for this app session, rather than letting remounts send another head change.
const unresolved = new Map<string, Pending>();
const listeners = new Map<string, Set<() => void>>();
const notify = (key: string) => listeners.get(key)?.forEach(listener => listener());
export function useVersionPublication(meetingId: string, kind: PublicationReview["kind"], locked: boolean, onVerified: () => Promise<void>) {
  const key = toastKey(meetingId, `${kind}-publication`);
  const [pending, setPending] = useState(() => unresolved.get(key) ?? null);
  const [checking, setChecking] = useState(false);
  const mounted = useRef(true), checkingRef = useRef(false), latest = useRef({ locked, onVerified });
  latest.current = { locked, onVerified };
  useEffect(() => {
    mounted.current = true;
    const update = () => { const value = unresolved.get(key); setPending(value ? { ...value } : null); };
    const owners = listeners.get(key) ?? new Set(); listeners.set(key, owners); owners.add(update); update();
    return () => { mounted.current = false; owners.delete(update); if (!owners.size) listeners.delete(key); };
  }, [key]);
  const title = kind === "analysis" ? "文字起こし" : "議事録";
  const accept = (attempt: Pending) => {
    if (unresolved.get(key) !== attempt) throw Error("確認対象の採用・復元操作が変わりました。");
    unresolved.delete(key); notify(key);
    toasts.resolve(toastKey(key, "unknown"));
    toasts.show({ key, level: "success", eventId: publicationEvent(attempt.review), message: `${title}${attempt.review.restoring ? "の直前版へ戻しました" : "候補を採用しました"}（#${attempt.review.revisionNo}）。保存済み編集は版ごとに保持しています。` });
  };
  const fail = (attempt: Pending, cause: unknown) => {
    attempt.error = String(cause); attempt.active = false;
    notify(key);
    toasts.show({ key: toastKey(key, "unknown"), level: "error", message: `${title}の採用・復元結果を確認できません。確認した版を保持しています`, eventId: publicationEvent(attempt.review) });
  };
  const apply = async (review: PublicationReview, command: string, args: unknown) => {
    if (review.meetingId !== meetingId || review.kind !== kind) throw Error("採用・復元の会議または工程が一致しません。");
    const allowed = kind === "analysis" ? review.restoring ? "restore_analysis_version" : "adopt_analysis_candidate" : review.restoring ? "restore_minutes_version" : "adopt_minutes_candidate";
    if (command !== allowed) throw Error("採用・復元の操作が一致しません。");
    const prior = unresolved.get(key);
    // Explicit re-confirmation may retry only the identical old CAS request.
    // Even a late first commit makes its old lock fail; restore cannot toggle
    // back by silently fetching a fresh lock/version for an unknown attempt.
    if (prior && (prior.active || !samePublicationRequest(prior, review, command, args))) throw Error("先の結果が未確定です。確認情報を変更せず、結果だけを確認してください。");
    const attempt: Pending = prior ?? { review: structuredClone(review), command, argsKey: JSON.stringify(args), error: "保存結果を確認しています。", active: true };
    attempt.active = true; attempt.error = "保存結果を確認しています。";
    unresolved.set(key, attempt); notify(key);
    try { const result = await invoke<unknown>(command, args as Record<string, unknown>); assertPublicationReceipt(review, result); accept(attempt); }
    catch (cause) { fail(attempt, cause); throw cause; }
  };
  const check = async () => {
    const attempt = unresolved.get(key);
    if (!attempt || attempt.active || checkingRef.current || latest.current.locked) return;
    checkingRef.current = true; setChecking(true);
    try {
      // Read only. In particular, never resend restore: it is not idempotent.
      const state = await invoke<unknown>(kind === "analysis" ? "get_analysis_versions" : "get_minutes_versions", { meetingId });
      publicationFromHead(attempt.review, state); accept(attempt);
      if (mounted.current) await latest.current.onVerified();
    } catch (cause) { if (unresolved.get(key) === attempt) fail(attempt, cause); }
    finally { checkingRef.current = false; if (mounted.current) setChecking(false); }
  };
  const status = pending && <IssueStatus label={`${title}の採用・復元結果`} message={`対象 #${pending.review.revisionNo} · 版ID ${pending.review.id}\n${pending.error}\n通知を閉じても確認情報を保持します。結果確認は変更命令を送信しません。`}>
    <button type="button" className="small-secondary" disabled={locked || checking || pending.active} onClick={() => void check()}>{checking ? "結果を確認中" : "採用・復元の結果だけを確認"}</button>
  </IssueStatus>;
  return { pending: !!pending, inFlight: !!pending?.active, apply, status };
}
