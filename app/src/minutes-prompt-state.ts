import type { VersionedDraft } from "./draft-save.ts";

export interface MinutesPromptPreferences {
  schemaVersion: 1;
  extractPrompt: string;
  mergePrompt: string;
}
export interface MinutesPromptSnapshot extends VersionedDraft<MinutesPromptPreferences> {
  defaults: MinutesPromptPreferences;
}
export function promptValidation(value: MinutesPromptPreferences): string {
  for (const [label, text] of [["作成用", value.extractPrompt], ["統合用", value.mergePrompt]]) {
    if (!text.trim() || text.includes("\0") || new TextEncoder().encode(text).length > 16 * 1024) {
      return `${label}プロンプトは空白のみを避け、NUL文字を含めずUTF-8で16KiB以内にしてください。`;
    }
  }
  return "";
}
export const resetPrompt = (value: MinutesPromptPreferences, defaults: MinutesPromptPreferences, field: "extractPrompt" | "mergePrompt"): MinutesPromptPreferences => ({ ...value, [field]: defaults[field] });
