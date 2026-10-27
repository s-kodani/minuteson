import { useId } from "react";

export function MeetingDeletionMenu({ meeting, recoveryRunning, deletion, audioDeletion, onOpenJobs, onOpenRecording }: {
  recoveryRunning?: boolean;
  onOpenJobs?: () => void; onOpenRecording?: () => void;
  meeting: { id: string; title: string; lifecycle: string };
  deletion: { canReviewMeetingId: (id: string) => boolean; meetingReviewReason: (id: string) => string | null; meetingBlockers: (id: string) => {kind:string}[]; retryAvailability: () => Promise<void>; reviewMeeting: (id: string) => unknown };
  audioDeletion: { canReviewNewId: (id: string) => boolean; meetingReviewReason: (id: string) => string | null; reviewNew: (id: string) => unknown };
}) {
  const note = useId();
  const hasAudio = ["recorded", "imported"].includes(meeting.lifecycle);
  const nativeReason = deletion.meetingReviewReason(meeting.id);
  const reason = recoveryRunning && nativeReason && deletion.meetingBlockers(meeting.id).some(b => b.kind === "recording") ? "録音を回復中です。完了するまで削除できません。" : nativeReason;
  const blockers = deletion.meetingBlockers(meeting.id);
  const audioReason = hasAudio ? audioDeletion.meetingReviewReason(meeting.id) : null;
  const reasons = [...new Set([reason, audioReason && audioReason !== reason ? `音声だけの削除：${audioReason}` : audioReason].filter((value): value is string => !!value))];
  return <>
    {hasAudio && <span className="audio-deletion-menu-item" title={audioReason ?? undefined}><button type="button" role="menuitem" className="deletion-trigger" aria-label={`${meeting.title}の音声だけを削除`}
      aria-describedby={audioReason ? note : undefined} onClick={() => void audioDeletion.reviewNew(meeting.id)} disabled={!audioDeletion.canReviewNewId(meeting.id)}>音声だけを削除</button></span>}
    <button type="button" role="menuitem" className="deletion-trigger" aria-label={`${meeting.title}を削除`}
      aria-describedby={reason ? note : undefined} onClick={() => void deletion.reviewMeeting(meeting.id)} disabled={!deletion.canReviewMeetingId(meeting.id)}>削除</button>
    {blockers.some(b => b.kind === "job") && onOpenJobs && <button type="button" role="menuitem" onClick={onOpenJobs}>ジョブ一覧で確認</button>}
    {blockers.some(b => b.kind === "recording") && onOpenRecording && <button type="button" role="menuitem" onClick={onOpenRecording}>録音画面で確認</button>}
    {reason && <button type="button" role="menuitem" onClick={() => void deletion.retryAvailability()}>削除可否を再確認</button>}
    {reasons.length > 0 && <p className="meeting-row-menu-note" id={note}>{reasons.join(" ")}</p>}
  </>;
}
