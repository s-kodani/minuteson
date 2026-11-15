import type { AppUpdateStatus } from "./generated/contracts";
import { validateAppUpdateStatus } from "./generated/validators.js";
export type { AppUpdateStatus };

export function checkedAppUpdateStatus(value: unknown): AppUpdateStatus {
  if (!validateAppUpdateStatus(value)) throw Error("更新確認の状態を読み取れません。");
  if (["available", "downloading", "ready", "installing"].includes(value.state) && !value.updateAvailable || value.state === "no_update" && value.updateAvailable
    || value.channel === "unsupported" && value.state !== "unsupported") throw Error("更新確認の状態が一致しません。");
  return value;
}
export function updateHeading(status: AppUpdateStatus): string {
  if (status.state === "downloading") return "更新をダウンロードしています";
  if (status.state === "ready") return "更新の準備ができました";
  if (status.state === "installing") return "更新を適用しています";
  if (status.state === "failed") return "更新を取得できませんでした";
  if (status.state === "checking") return "更新を確認しています";
  if (status.state === "unavailable") return "更新を確認できませんでした";
  if (status.state === "unsupported") return "この配布版は更新確認の対象外です";
  if (status.state === "not_checked") return "更新はまだ確認していません";
  if (status.updateAvailable) return status.availableVersion
    ? `バージョン${status.availableVersion}を利用できます` : "Microsoft Storeに更新があります";
  return status.channel === "windows_store" ? "Storeで利用可能な更新は見つかりませんでした" : "公開されている新版は見つかりませんでした";
}
export function updateSource(status: AppUpdateStatus): string {
  return status.channel === "windows_store" ? "Microsoft Store" : status.channel === "macos_release" ? "GitHub Releases" : "対象外";
}
export function updateInstructions(status: AppUpdateStatus): string {
  if (status.channel === "windows_store") return "起動中は更新の適用を延期します。録音・解析・保存を終え、アプリの「終了」から終了してください。";
  const version = status.availableVersion ? `バージョン${status.availableVersion}` : "更新";
  if (status.state === "ready") return `${version}を取得しました。録音・解析・保存が完了したら「再起動して更新」を押してください。`;
  if (status.state === "installing") return `${version}を適用しています。完了後にアプリを再起動します。`;
  return `${version}は自動で取得します。録音・解析・保存が完了したら「再起動して更新」を押してください。`;
}
