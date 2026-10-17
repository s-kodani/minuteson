export interface VadOptions {
  enabled: boolean;
  thresholdMilli: number;
  minSpeechDurationMs: number;
  minSilenceDurationMs: number;
  maxSpeechDurationS: number;
  speechPadMs: number;
  samplesOverlapMs: number;
}
export const defaultVadOptions: VadOptions = {
  enabled: true, thresholdMilli: 600, minSpeechDurationMs: 300, minSilenceDurationMs: 100,
  maxSpeechDurationS: 30, speechPadMs: 30, samplesOverlapMs: 100,
};
export function checkedVadOptions(value: unknown): VadOptions {
  const v = value as VadOptions;
  if (!v || typeof v !== "object" || Object.keys(v).length !== 7 || typeof v.enabled !== "boolean"
    || Object.keys(defaultVadOptions).some(k => !Object.hasOwn(v, k))
    || Object.entries(v).some(([k, n]) => k !== "enabled" && !Number.isSafeInteger(n))
    || v.thresholdMilli < 10 || v.thresholdMilli > 990 || v.thresholdMilli % 10 !== 0
    || v.minSpeechDurationMs < 1 || v.minSpeechDurationMs > 5000
    || v.minSilenceDurationMs < 1 || v.minSilenceDurationMs > 5000
    || v.maxSpeechDurationS < 1 || v.maxSpeechDurationS > 120
    || v.speechPadMs < 0 || v.speechPadMs > 1000
    || v.samplesOverlapMs < 0 || v.samplesOverlapMs > 1000 || v.samplesOverlapMs % 10 !== 0
    || v.minSpeechDurationMs > v.maxSpeechDurationS * 1000) {
    throw Error("VADの数値が入力範囲外か、最小発話時間が最大発話区間を超えています。");
  }
  return v;
}
export const sameVadOptions = (a: VadOptions, b: VadOptions) =>
  Object.keys(defaultVadOptions).every(k => a[k as keyof VadOptions] === b[k as keyof VadOptions]);
