import { errorMessage } from "./app-error.ts";

// Detailed filesystem diagnostics remain in the model preparation log.
const fileReadFailures = new Set([
  "モデルの親フォルダーにリンクまたは変更があります",
  "検査中にモデルの親フォルダーが変更されました",
  "検査中にモデルパッケージが変更されました",
]);

export function modelPreparationError(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : errorMessage(cause);
  if (!fileReadFailures.has(message)) return message;
  return "モデルを読み込めません。もう一度お試しください。";
}
