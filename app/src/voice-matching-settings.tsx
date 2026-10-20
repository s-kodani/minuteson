import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { IssueNotice } from "./toast";
import { matchingThreshold, type VoiceMatchingPreferences } from "./voice-matching-state";

type Snapshot = { id: string; payload: VoiceMatchingPreferences; lockVersion: number };
function checkedSnapshot(value: Snapshot): Snapshot {
  if (value.id !== "voice_matching" || !Number.isSafeInteger(value.lockVersion) || value.lockVersion < 0
    || Object.values(value.payload).length !== 2 || [value.payload.similarityThresholdMilli, value.payload.marginThresholdMilli].some(v => !Number.isInteger(v) || v < 0 || v > 1000 || v % 10 !== 0)) {
    throw Error("声の自動適用設定の保存状態を確認できません。");
  }
  return value;
}
export function VoiceMatchingSettings({ locked = false, embedded = false }: { locked?: boolean; embedded?: boolean }) {
  const [saved, setSaved] = useState<Snapshot | null>(null);
  const [similarity, setSimilarity] = useState("0.80"), [margin, setMargin] = useState("0.10");
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [uncertain, setUncertain] = useState(false);
  const inFlight = useRef(false), mounted = useRef(true);
  const accept = (s: Snapshot) => {
    setSaved(s);
    setSimilarity((s.payload.similarityThresholdMilli / 1000).toFixed(2));
    setMargin((s.payload.marginThresholdMilli / 1000).toFixed(2));
  };
  const load = async () => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true);
    try {
      const s = checkedSnapshot(await invoke<Snapshot>("get_voice_matching_preferences"));
      if (mounted.current) { accept(s); setUncertain(false); setError(""); }
    } catch (e) { if (mounted.current) setError(String(e)); }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  useEffect(() => { mounted.current = true; if (isTauri()) void load(); return () => { mounted.current = false; }; }, []);
  let payload: VoiceMatchingPreferences | null = null, validation = "";
  try { payload = { similarityThresholdMilli: matchingThreshold(similarity), marginThresholdMilli: matchingThreshold(margin) }; }
  catch (e) { validation = e instanceof Error ? e.message : String(e); }
  const changed = !!saved && !!payload && (saved.payload.similarityThresholdMilli !== payload.similarityThresholdMilli || saved.payload.marginThresholdMilli !== payload.marginThresholdMilli);
  const apply = async () => {
    if (!saved || !payload || locked || uncertain || inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try {
      const s = checkedSnapshot(await invoke<Snapshot>("save_voice_matching_preferences", { expectedLockVersion: saved.lockVersion, payload }));
      if (s.lockVersion !== saved.lockVersion + 1 || s.payload.similarityThresholdMilli !== payload.similarityThresholdMilli || s.payload.marginThresholdMilli !== payload.marginThresholdMilli) {
        throw Error("設定の保存結果が一致しません。現在の状態を確認してください。");
      }
      if (mounted.current) accept(s);
    } catch (e) { if (mounted.current) { setError(String(e)); setUncertain(true); } }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  return <section className={embedded ? "voice-matching-settings ai-settings-group" : "analysis-card voice-matching-settings"} aria-label="登録した声の自動適用">
    {embedded ? <h3>登録した声の自動適用</h3> : <h2>登録した声の自動適用</h2>}
    <p>両方の条件を満たした場合、名前が未設定の話者に登録した名前を自動適用します。類似度は確率ではありません。登録が1人の場合は類似度の下限だけで判定します。</p>
    <fieldset className="vad-fields" disabled={locked || busy || !saved || uncertain}>
      <label><span>類似度の下限</span><input type="number" min="0" max="1" step="0.01" value={similarity} onChange={e => setSimilarity(e.target.value)} /></label>
      <label><span>次点との差の下限</span><input type="number" min="0" max="1" step="0.01" value={margin} onChange={e => setMargin(e.target.value)} /></label>
    </fieldset>
    <p>次に開始する照合から適用します。既存会議は「登録した声と照合」で照合できます。</p>
    <IssueNotice scope="voice-matching-settings" type="settings" label="声の自動適用設定" message={error || validation} />
    <div className="analysis-actions">
      <button type="button" disabled={locked || busy || !changed || !!validation || uncertain} onClick={() => void apply()}>適用</button>
      {(uncertain || !saved) && <button type="button" className="small-secondary" disabled={busy || locked} onClick={() => void load()}>現在の状態を確認</button>}
      <span role="status">{busy ? "保存状態を確認中" : uncertain ? "保存要確認" : !saved ? "読込待ち" : changed || validation ? "未適用" : "保存済み"}</span>
    </div>
  </section>;
}
