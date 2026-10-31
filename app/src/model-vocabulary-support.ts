import type { ModelPlacementEntry } from "./model-inspection-state.ts";

export type VocabularyBackend = { id: string; vocabularyDescription: string };

/** Resolve the inspected copy, independently of the currently applied model. */
export function modelVocabularySupport(
  model: Pick<ModelPlacementEntry, "engine" | "backendId">,
  backends: readonly VocabularyBackend[],
): string | null {
  if (model.engine !== "whisper.cpp" && model.engine !== "asr") return null;
  const backendId = model.engine === "whisper.cpp" ? "whisper.cpp" : model.backendId;
  const description = backends.find(backend => backend.id === backendId)?.vocabularyDescription;
  return typeof description === "string" && description.trim()
    ? description
    : "対応情報を確認できません";
}
