import { displayNativePath } from "./native-path.ts";
import { bytesLabel } from "./import-state.ts";
import type { DataSnapshot } from "./data-operation-state.ts";

/** A committed receipt and its own fixed destination, never the latest selection. */
export function backupNotification(snapshot: DataSnapshot): { message: string; details: string } | null {
  const receipt = snapshot.backupReceipt;
  if (snapshot.kind !== "backup" || snapshot.phase !== "completed" || snapshot.busy || !receipt) return null;
  const destination = displayNativePath(snapshot.backupPreview?.destination);
  return {
    message: "バックアップを作成しました。",
    details: [destination ? `保存先: ${destination}` : "保存先の表示を確認できません。",
      `会議 ${receipt.meetingCount} 件 · ${bytesLabel(receipt.size)}`,
      receipt.deletedAudioMeetingCount > 0 ? `音声削除済みの会議 ${receipt.deletedAudioMeetingCount} 件・音声資産 ${receipt.deletedAudioAssetCount} 件は、保存済み文書と音声の欠落情報を保存しました。音声本体は含みません。追加復元後も削除済み音声の再生・再解析はできません。` : "",
    ].filter(Boolean).join("\n"),
  };
}
