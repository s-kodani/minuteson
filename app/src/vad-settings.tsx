import { InfoPopover } from "./info-popover";
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
type Field = typeof fields[number]["key"];
const draftFor = (v: VadOptions) => Object.fromEntries(fields.map(f => [f.key, f.scale === 1000 ? (v[f.key] / f.scale).toFixed(2) : String(v[f.key])]));
export function VadSettings({ value, available, disabled, onApply, overlapAvailable = true, defaults = defaultVadOptions }: {
  overlapAvailable?: boolean; defaults?: VadOptions;
  value: VadOptions; available: boolean; disabled: boolean; onApply: (options: VadOptions) => void;
}) {
  const [draft, setDraft] = useState(() => draftFor(value)), [enabled, setEnabled] = useState(value.enabled);
  const [touched, setTouched] = useState(false);
  const id = useId();
  useEffect(() => { setDraft(draftFor(value)); setEnabled(value.enabled); setTouched(false); }, [value]);
  const errors: Partial<Record<Field, string>> = {};
  const numbers: Partial<Record<Field, number>> = {};
  for (const field of fields) {
    if (field.key === "samplesOverlapMs" && !overlapAvailable) {
      numbers[field.key] = 0;
      continue;
    }
    const raw = draft[field.key], n = Number(raw);
    if (!raw.trim() || !Number.isFinite(n) || n < field.min || n > field.max || Math.abs(n / field.step - Math.round(n / field.step)) > 1e-6) {
      errors[field.key] = `${field.min}～${field.max}${field.unit}の範囲で、${field.step}${field.unit}刻みで入力してください。`;
    } else {
      numbers[field.key] = Math.round(n * field.scale);
    }
  }
  if (numbers.minSpeechDurationMs !== undefined && numbers.maxSpeechDurationS !== undefined
    && numbers.minSpeechDurationMs > numbers.maxSpeechDurationS * 1000) {
    errors.minSpeechDurationMs = "最小発話時間は最大発話区間以下にしてください。";
  }
  const options = Object.keys(errors).length ? null : checkedVadOptions({ enabled, ...numbers });
  const changed = options !== null && !sameVadOptions(options, value);
  return <section className="vad-settings" aria-label="発話区間検知（VAD）">
    <div className="heading-with-info"><label className="vad-enabled"><input type="checkbox" checked={enabled} disabled={disabled || !available} onChange={e => { setEnabled(e.target.checked); setTouched(true); }} />発話区間検知（VAD）</label><InfoPopover label="発話区間検知（VAD）">無音や雑音を除外して文字起こしします。短い発話や小声が除外される場合があります。変更は次に開始する処理から適用します。</InfoPopover></div>
    {!available && <p>モデル選択欄の「VADを追加して使用」から導入できます。</p>}
    <div className="vad-detail-fields">
        <fieldset className="vad-fields">
          {fields.filter(f => overlapAvailable || f.key !== "samplesOverlapMs").map(f => {
            const fieldId = `${id}-${f.key}`;
            const error = touched && errors[f.key];
            return <div className="info-field" key={f.key}>
              <div className="heading-with-info"><label htmlFor={fieldId}>{f.label}{f.unit && `（${f.unit}）`}</label><InfoPopover label={f.label}>{f.help}</InfoPopover></div>
              <input id={fieldId} type="number" disabled={disabled || !available || !enabled} min={f.min} max={f.max} step={f.step} value={draft[f.key]} aria-describedby={error ? `${fieldId}-help ${fieldId}-error` : `${fieldId}-help`} aria-invalid={!!error} onChange={e => { setDraft(d => ({ ...d, [f.key]: e.target.value })); setTouched(true); }} />
              <span className="sr-only" id={`${fieldId}-help`}>{f.help}</span>
              {error && <small id={`${fieldId}-error`} className="settings-field-error">{error}</small>}
            </div>;
          })}
        </fieldset>
    </div>
    <div className="analysis-actions vad-actions">
      <button type="button" disabled={disabled || !available || !changed} onClick={() => options && onApply(options)}>VAD設定を適用</button>
      <button type="button" className="small-secondary" disabled={disabled || !available || !enabled} onClick={() => { setDraft(draftFor(defaults)); setTouched(true); }}>標準値に戻す</button>
    </div>
  </section>;
}
