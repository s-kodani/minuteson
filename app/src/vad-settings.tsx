import { ModelIssue } from "./model-issue";
import { useEffect, useId, useState } from "react";
import { checkedVadOptions, defaultVadOptions, sameVadOptions, type VadOptions } from "./vad-state";

const fields = [
  { key: "thresholdMilli", label: "発話判定の閾値", unit: "", scale: 1000, min: 0.01, max: 0.99, step: 0.01, help: "高くすると雑音を除外しやすくなりますが、小声も除外されやすくなります。" },
  { key: "minSpeechDurationMs", label: "最小発話時間", unit: "ms", scale: 1, min: 1, max: 5000, step: 1, help: "これより短い発話は除外します。長くすると短い相づちも除外されやすくなります。" },
  { key: "minSilenceDurationMs", label: "区切りとする最小無音時間", unit: "ms", scale: 1, min: 1, max: 5000, step: 1, help: "この長さの無音を発話の区切りとして扱います。" },
  { key: "maxSpeechDurationS", label: "最大発話区間", unit: "秒", scale: 1, min: 1, max: 120, step: 1, help: "長い発話区間を分割する目安です。" },
  { key: "speechPadMs", label: "発話前後の余白", unit: "ms", scale: 1, min: 0, max: 1000, step: 1, help: "発話の前後に余白を付け、語頭や語尾の欠落を抑えます。" },
  { key: "samplesOverlapMs", label: "区間の重なり", unit: "秒", scale: 1000, min: 0, max: 1, step: 0.01, help: "隣り合う発話区間をつなぐ際の重なりです。" },
] as const;
const draftFor = (v: VadOptions) => Object.fromEntries(fields.map(f => [f.key, f.scale === 1000 ? (v[f.key] / f.scale).toFixed(2) : String(v[f.key])]));
export function VadSettings({ value, available, disabled, onApply }: {
  value: VadOptions; available: boolean; disabled: boolean; onApply: (options: VadOptions) => void;
}) {
  const [draft, setDraft] = useState(() => draftFor(value)), [enabled, setEnabled] = useState(value.enabled);
  const [touched, setTouched] = useState(false);
  const id = useId();
  useEffect(() => { setDraft(draftFor(value)); setEnabled(value.enabled); setTouched(false); }, [value]);
  let options: VadOptions | null = null, error = "";
  try {
    const numbers = Object.fromEntries(fields.map(f => {
      const raw = draft[f.key], n = Number(raw);
      if (raw.trim() === "" || !Number.isFinite(n) || n < f.min || n > f.max || Math.abs(n / f.step - Math.round(n / f.step)) > 1e-6) throw Error("各項目の入力範囲と刻みを確認してください。");
      return [f.key, Math.round(n * f.scale)];
    }));
    options = checkedVadOptions({ enabled, ...numbers });
  } catch (cause) { error = cause instanceof Error ? cause.message : String(cause); }
  const changed = options !== null && !sameVadOptions(options, value);
  return <section className="vad-settings" aria-label="発話区間検知（VAD）">
    <label className="vad-enabled"><input type="checkbox" checked={enabled} disabled={disabled || !available} onChange={e => { setEnabled(e.target.checked); setTouched(true); }} />発話区間検知（VAD）</label>
    <p>無音や雑音を除外して文字起こしします。短い発話や小声が除外される場合があります。変更は次に開始する処理から適用します。</p>
    {!available && <p>モデル選択欄の「VADを追加して使用」から導入できます。</p>}
    <div className="vad-detail-fields">
        <fieldset className="vad-fields" disabled={disabled || !available || !enabled}>
          {fields.map(f => <label key={f.key} htmlFor={`${id}-${f.key}`}><span>{f.label}{f.unit && `（${f.unit}）`}</span>
            <input id={`${id}-${f.key}`} type="number" aria-label={`${f.label}${f.unit ? `（${f.unit}）` : ""}`} min={f.min} max={f.max} step={f.step} value={draft[f.key]} aria-describedby={`${id}-${f.key}-help`} aria-invalid={touched && !!error} onChange={e => { setDraft(d => ({ ...d, [f.key]: e.target.value })); setTouched(true); }} />
            <span className="vad-field-help" id={`${id}-${f.key}-help`}>{f.help}</span>
          </label>)}
        </fieldset>
    </div>
    <ModelIssue label="VAD設定" message={touched ? error : null} />
    <div className="analysis-actions vad-actions">
      <button type="button" disabled={disabled || !available || !changed || !!error} onClick={() => options && onApply(options)}>VAD設定を適用</button>
      <button type="button" className="small-secondary" disabled={disabled || !available || !enabled} onClick={() => { setDraft(draftFor(defaultVadOptions)); setTouched(true); }}>標準値に戻す</button>
    </div>
  </section>;
}
