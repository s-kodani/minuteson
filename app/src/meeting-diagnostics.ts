export interface MeetingDiagnostic {
  reason: string;
  message: string;
}

interface DiagnosticPresentation {
  label: string;
  description: string;
  priority: number;
}

const retainedDocuments = "音声の再生・再解析はできません。保存済みの文字起こし・議事録は引き続き閲覧・編集できます。";
const retainedReadOnlyDocuments = "音声の再生・再解析はできません。保存済みの文字起こし・議事録は保持しています。";
const audioDocumentReasons = new Set(["audio_missing", "audio_unreadable", "audio_unavailable"]);
const restoredParts = "復元できたデータは引き続き利用できます。";
const presentations: Record<string, DiagnosticPresentation> = {
  meeting_row_invalid: {
    label: "会議データを利用できません", priority: 100,
    description: "会議情報を正常に読み取れないため、編集・解析を停止しています。この画面から修復する機能はありません。不要な場合は会議の操作メニューから「削除」を選んでください。",
  },
  deletion_pending: {
    label: "会議の削除が未完了", priority: 90,
    description: "会議の削除が完了したことを確認できないため、編集・解析を停止しています。この画面から修復や削除のやり直しはできません。",
  },
  salvage_record_invalid: {
    label: "復元記録を読み取れません", priority: 80,
    description: "この会議の復元記録を読み取れないため、一部のデータや操作を利用できません。不要な場合は会議の操作メニューから「削除」を選んでください。",
  },
  audio_deletion_pending: {
    label: "音声の削除が未完了", priority: 70,
    description: "音声の削除が完了したことを確認できないため、音声の再生・再解析を停止しています。保存済みの文字起こし・議事録は保持しています。この画面から音声の削除をやり直すことはできません。",
  },
  audio_missing: { label: "音声なし", priority: 60, description: "保存済みの音声ファイルが見つかりません。" },
  audio_unreadable: { label: "音声を利用できません", priority: 60, description: "保存済みの音声ファイルを読み取れません。" },
  audio_unavailable: { label: "音声を利用できません", priority: 60, description: "音声を復元できませんでした。" },
  audio_recovery: { label: "音声保存エラー", priority: 50, description: "音声の保存が完了していません。" },
  processing_job: { label: "解析ジョブを再開できません", priority: 40, description: "保存済みの解析ジョブを再開できません。ジョブ一覧で処理の状態を確認してください。" },
  salvage_meeting: { label: "会議情報の一部を復元できませんでした", priority: 30, description: `会議名などの一部を既定値に置き換えて復元しました。${restoredParts}` },
  salvage_transcript: { label: "文字起こしを復元できませんでした", priority: 30, description: `保存済みの文字起こしを復元できませんでした。${restoredParts}` },
  salvage_draft: { label: "文字起こしの下書きを復元できませんでした", priority: 30, description: `保存済みの文字起こしの下書きを復元できませんでした。${restoredParts}` },
  salvage_minutes: { label: "議事録を復元できませんでした", priority: 30, description: `保存済みの議事録を復元できませんでした。${restoredParts}` },
  salvage_vocabulary: { label: "会議用語を復元できませんでした", priority: 30, description: `保存済みの会議用語を復元できませんでした。${restoredParts}` },
};

// Presentation only: diagnostics must not grant write, recovery or deletion permission.
export function meetingDiagnosticPresentation(item: MeetingDiagnostic, { documentEditingBlocked = false }: { documentEditingBlocked?: boolean } = {}): DiagnosticPresentation {
  const presentation = Object.hasOwn(presentations, item.reason) ? presentations[item.reason]
    : { label: "会議データの一部を利用できません", description: item.message, priority: 20 };
  return audioDocumentReasons.has(item.reason)
    ? { ...presentation, description: presentation.description + (documentEditingBlocked ? retainedReadOnlyDocuments : retainedDocuments) }
    : presentation;
}

export function meetingDiagnosticSummary(items: readonly MeetingDiagnostic[] | undefined): (DiagnosticPresentation & { reason: string }) | null {
  let summary: (DiagnosticPresentation & { reason: string }) | null = null;
  for (const item of items ?? []) {
    const presentation = meetingDiagnosticPresentation(item);
    if (!summary || presentation.priority > summary.priority) summary = { ...presentation, reason: item.reason };
  }
  return summary;
}
