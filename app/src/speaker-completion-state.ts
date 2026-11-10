export { matchingThreshold, matchingThresholdInput } from "./voice-matching-state.ts";
export type CompletionPreferences = {similarityThresholdMilli:number; marginThresholdMilli:number};

export function identificationText(value: import("./transcript-state.ts").SpeakerIdentification | null | undefined) {
  if (!value || !Number.isInteger(value.similarityMilli) || Math.abs(value.similarityMilli)>1000
    || value.runnerUpMilli!==null && (!Number.isInteger(value.runnerUpMilli) || value.runnerUpMilli < -1000 || value.runnerUpMilli > value.similarityMilli)) return null;
  const reason = ({ nearest_tie:"候補が同点", nearest_below_threshold:"類似度または次点との差が基準未達",
    nearest_different_winners:"発言本体と前後付きの候補が不一致", nearest_other_speaker_in_context:"前後に別話者の区間あり",
    nearest_feature_unavailable:"前後付きの特徴量を取得できず", applied:"確認基準を満たしています" } as Record<string,string>)[value.reason] ?? "類似度1位を採用";
  return {label:`自動識別${value.needsReview?"・要確認":""} · 類似度 ${(value.similarityMilli/1000).toFixed(3)}`,
    detail:`${reason}。${value.runnerUpMilli===null?"比較候補は1人です。":`次点との差 ${((value.similarityMilli-value.runnerUpMilli)/1000).toFixed(3)}。`}類似度は確率ではありません。`,needsReview:value.needsReview};
}
