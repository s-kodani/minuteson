import type { ModelPlacementEntry } from "./model-inspection-state.ts";
import type { ModelProfileView } from "./model-profile-state.ts";

export function installedModels(entries: ModelPlacementEntry[]) {
  return entries.filter(entry => entry.placementRecorded && entry.bodyRemoved !== true);
}

export function modelUsage(entry: ModelPlacementEntry, profiles: ModelProfileView[] | null): string {
  if (entry.bodyRemoved === true) return "本体なし";
  if (!entry.placementRecorded) return "取り込み未完了";
  if (!profiles) return "利用状態を確認できません";
  const using = profiles.filter(profile => entry.compatibility?.state === "succeeded" && profile.components.some(component => component.packageId === entry.packageId && component.engine === entry.engine && component.checkOperationId === entry.compatibility?.operationId && component.settingsMatch && component.compatibilityCurrent));
  if (using.length) return using.map(profile => profile.name === "standard" ? "標準で使用" : "高精度で使用").join("・");
  return entry.compatibility?.state === "succeeded" ? "動作確認済み・未選択" : entry.compatibility?.state === "failed" ? "動作確認に失敗" : "動作確認待ち";
}
