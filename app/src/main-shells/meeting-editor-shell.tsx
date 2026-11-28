import { lazy, useState, type ComponentProps, type FormEvent, type ReactNode, type RefObject } from "react";
import { IconButton } from "../action-icon";
import { CommonModal } from "../common-modal";
import { RowMenu } from "../row-menu";
import type { useSavedNavigation } from "../saved-navigation";
import { IssueNotice } from "../toast";
import { VocabularyEditor } from "../vocabulary";
import { DeferredPanel } from "./deferred-panel";

const TranscriptEditor = lazy(() => import("../transcript").then(module => ({ default: module.TranscriptEditor })));
const AnalysisPanel = lazy(() => import("../analysis").then(module => ({ default: module.AnalysisPanel })));
const MinutesPanel = lazy(() => import("../minutes").then(module => ({ default: module.MinutesPanel })));

export type DetailTab = "transcript" | "minutes";

export interface MeetingEditorShellProps {
  meeting: { id: string; title: string; recorded: boolean; audioDeleted: boolean | null } | null;
  detail: boolean;
  activeTab: DetailTab;
  minutesMounted: boolean;
  recordingActive: boolean;
  locks: { write: boolean; editing: boolean; navigation: boolean; analysis: boolean };
  analysis: {
    epoch: number;
    hasTranscript: boolean;
    publicationStatus: ComponentProps<typeof AnalysisPanel>["publicationReadStatus"];
    replace: ComponentProps<typeof AnalysisPanel>["onReplaceGenerated"];
    headChanged: (id: string, read: Parameters<ComponentProps<typeof AnalysisPanel>["onHeadChanged"]>[0]) => Promise<void>;
    loaded: (id: string, present: boolean, readOnly: boolean) => void;
    consolidated: (id: string) => Promise<void>;
  };
  playbackHost: HTMLDivElement | null;
  withSavedEdits: ReturnType<typeof useSavedNavigation>["withSavedEdits"];
  library: ReactNode;
}

export function MeetingEditorShell({
  meeting, detail, activeTab, minutesMounted, recordingActive, locks, analysis, playbackHost, withSavedEdits, library,
}: MeetingEditorShellProps) {
  const [transcriptActionsHost, setTranscriptActionsHost] = useState<HTMLDivElement | null>(null);
  const [transcriptEditorActionsHost, setTranscriptEditorActionsHost] = useState<HTMLDivElement | null>(null);
  const showingTranscript = activeTab === "transcript";

  return <>
    {meeting ? <DeferredPanel visible={detail && showingTranscript}>
      <AnalysisPanel actionsHost={transcriptActionsHost} key={meeting.id} meetingId={meeting.id} meetingTitle={meeting.title}
        visible={detail && showingTranscript} hasTranscript={analysis.hasTranscript} recorded={meeting.recorded}
        audioDeleted={meeting.audioDeleted} recordingActive={recordingActive} locked={locks.analysis}
        withSavedEdits={action => withSavedEdits(action, "transcription")} onReplaceGenerated={analysis.replace}
        publicationReadStatus={analysis.publicationStatus} onHeadChanged={read => analysis.headChanged(meeting.id, read)} />
    </DeferredPanel> : null}
    {library}
    {meeting && <div className="workspace-tab-panel" id="meeting-panel-transcript" role="tabpanel" aria-labelledby="meeting-tab-transcript" hidden={!showingTranscript}>
      <div className="card-head transcript-primary-heading">
        <h2>{analysis.hasTranscript ? "発言一覧" : "文字起こし"}</h2>
        <div className="plain-minutes-primary-actions transcript-primary-actions">
          <div ref={setTranscriptEditorActionsHost} className="transcript-editor-actions" />
          <VocabularyEditor compact key={meeting.id} meetingId={meeting.id} meetingTitle={meeting.title} locked={locks.write} />
          <div ref={setTranscriptActionsHost} className="transcript-generation-host" />
        </div>
      </div>
      <DeferredPanel><TranscriptEditor actionsHost={transcriptEditorActionsHost} key={meeting.id}
        sourceEpoch={analysis.epoch} meetingId={meeting.id} meetingTitle={meeting.title}
        writeProtected={locks.editing} navigationLocked={locks.navigation} playbackHost={playbackHost} visible={showingTranscript}
        onAnalysisLoaded={(present, readOnly) => analysis.loaded(meeting.id, present, readOnly)}
        onConsolidated={() => analysis.consolidated(meeting.id)} /></DeferredPanel>
    </div>}
    {meeting && <div className="workspace-tab-panel" id="meeting-panel-minutes" role="tabpanel" aria-labelledby="meeting-tab-minutes" hidden={activeTab !== "minutes"}>
      {minutesMounted && <DeferredPanel><MinutesPanel key={meeting.id} meetingId={meeting.id} meetingTitle={meeting.title}
        sourceEpoch={analysis.epoch} recordingActive={recordingActive} locked={locks.write}
        navigationLocked={locks.navigation} withSavedEdits={withSavedEdits} visible={activeTab === "minutes"} /></DeferredPanel>}
    </div>}
  </>;
}

export interface MeetingEditorHeaderProps {
  meeting: { id: string; title: string; createdAt: number };
  meetingError: string;
  activeTab: DetailTab;
  navigationLocked: boolean;
  onSwitchTab: (tab: DetailTab) => void;
  rename: { opener: RefObject<HTMLButtonElement | null>; locked: boolean; open: () => void };
  menu: { content: ReactNode; onOpenChange: (open: boolean) => void };
}

export function MeetingEditorHeader({ meeting, meetingError, activeTab, navigationLocked, onSwitchTab, rename, menu }: MeetingEditorHeaderProps) {
  return <>
    <div className="meeting-detail-status">
      <div><div className="meeting-title-row">
        <h1 title={meeting.title}>{meeting.title}</h1>
        <IconButton ref={rename.opener} label="会議名を変更" icon="edit" disabled={rename.locked} onClick={rename.open} />
      </div><p className="lead">作成 {new Date(meeting.createdAt).toLocaleString("ja-JP")}</p></div>
      <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} />
    </div>
    <div className="meeting-detail-navigation">
      <div className="meeting-detail-tabs" role="tablist" aria-label="会議詳細">
        {(["transcript", "minutes"] as const).map(tab => <button key={tab} type="button" role="tab" id={`meeting-tab-${tab}`}
          aria-controls={`meeting-panel-${tab}`} aria-selected={activeTab === tab} tabIndex={activeTab === tab ? 0 : -1}
          data-screen-navigation disabled={navigationLocked} onClick={() => onSwitchTab(tab)} onKeyDown={event => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
            const target = event.key === "Home" ? "transcript" : event.key === "End" ? "minutes" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "transcript" ? "minutes" : "transcript" : null;
            if (target) { event.preventDefault(); document.getElementById(`meeting-tab-${target}`)?.focus(); }
          }}>{tab === "transcript" ? "文字起こし" : "議事録"}</button>)}
      </div>
      <div className="meeting-detail-actions"><RowMenu label="会議の操作" key={meeting.id} disabled={navigationLocked} onOpenChange={menu.onOpenChange}>{menu.content}</RowMenu></div>
    </div>
  </>;
}

export interface MeetingRenameDialogProps {
  open: boolean;
  busy: boolean;
  locked: boolean;
  title: string;
  opener: RefObject<HTMLButtonElement | null>;
  onDismiss: () => void;
  onTitleChange: (title: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}

export function MeetingRenameDialog({ open, busy, locked, title, opener, onDismiss, onTitleChange, onSubmit }: MeetingRenameDialogProps) {
  return <CommonModal open={open} blocked={busy} onDismiss={onDismiss} restoreTarget={() => opener.current}
    className="meeting-rename-dialog" aria-labelledby="meeting-rename-title" dismissLabel="閉じる">
    <h2 id="meeting-rename-title">会議名を変更</h2>
    <form onSubmit={onSubmit}>
      <label htmlFor="rename-title">会議名<input id="rename-title" value={title} maxLength={200} onChange={event => onTitleChange(event.target.value)} disabled={busy || locked} /></label>
      <div className="dialog-actions"><button type="button" className="secondary" disabled={busy} onClick={onDismiss}>キャンセル</button><button type="submit" disabled={busy || locked || !title.trim()}>保存</button></div>
    </form>
  </CommonModal>;
}
