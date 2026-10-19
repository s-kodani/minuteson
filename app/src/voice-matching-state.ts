export type VoiceMatchingPreferences = { similarityThresholdMilli: number; marginThresholdMilli: number };
export const defaultVoiceMatchingPreferences: VoiceMatchingPreferences = {similarityThresholdMilli:800,marginThresholdMilli:100};
export function matchingThreshold(raw: string): number {
  const value=Number(raw);
  if (!raw.trim()||!Number.isFinite(value)||value<0||value>1||Math.abs(value*100-Math.round(value*100))>1e-6) throw Error("閾値は0.00～1.00、0.01刻みで設定してください。");
  return Math.round(value*1000);
}
