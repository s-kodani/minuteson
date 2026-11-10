import type { VersionedDraft } from "./draft-save.ts";

export interface MinutesPromptPreferences {
  schemaVersion: 1;
  extractPrompt: string;
  mergePrompt: string;
}
export interface MinutesPromptSnapshot extends VersionedDraft<MinutesPromptPreferences> {
  defaults: MinutesPromptPreferences;
}
function promptFieldError(text: string, label: string): string {
  if (!text.trim()) return `${label}を入力してください。`;
  if (text.includes("\0")) return `${label}に使えない文字が含まれています。入力を確認してください。`;
  if (new TextEncoder().encode(text).length > 16 * 1024) return `${label}を16,384バイト以内に短くしてください。`;
  return "";
}
export function promptFieldErrors(value: MinutesPromptPreferences) {
  return {
    extractPrompt: promptFieldError(value.extractPrompt, "作成用プロンプト"),
    mergePrompt: promptFieldError(value.mergePrompt, "統合用プロンプト"),
  };
}
export function promptValidation(value: MinutesPromptPreferences): string {
  const errors = promptFieldErrors(value);
  return errors.extractPrompt || errors.mergePrompt;
}
export const resetPrompt = (value: MinutesPromptPreferences, defaults: MinutesPromptPreferences, field: "extractPrompt" | "mergePrompt"): MinutesPromptPreferences => ({ ...value, [field]: defaults[field] });
