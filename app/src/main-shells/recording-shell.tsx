import type { ComponentProps, ReactNode } from "react";
import { RecordingVocabulary } from "../recording-vocabulary";
import { RecordingWorkspace } from "../recording-workspace";
import { IssueNotice } from "../toast";
import { VocabularyEditor } from "../vocabulary";
import type { VocabularyEntry } from "../vocabulary-state";

export interface RecordingShellProps {
  preparation: (NonNullable<ComponentProps<typeof RecordingWorkspace>["preparation"]> & {
    meetingVocabulary: VocabularyEntry[];
  }) | null;
  meeting: { id: string; title: string; canEditVocabulary: boolean } | null;
  snapshot: ComponentProps<typeof RecordingWorkspace>["snapshot"];
  owner: string | null;
  locked: boolean;
  vocabularyLocked: boolean;
  canStart: boolean;
  canStop: boolean;
  actions: {
    setTitle: (title: string) => void;
    setVocabulary: (items: VocabularyEntry[]) => void;
    start: () => void;
    stop: (source: HTMLButtonElement) => void;
    openSettings: () => void;
    confirmResult: () => void;
  };
  inputs: ReactNode;
  activeMicrophone: ReactNode;
  test: ReactNode;
  errors: { capture: string; confirmation: string; meeting: string };
}

export function RecordingShell({
  preparation, meeting, snapshot, owner, locked, vocabularyLocked, canStart, canStop,
  actions, inputs, activeMicrophone, test, errors,
}: RecordingShellProps) {
  return <RecordingWorkspace
    preparation={preparation}
    title={meeting?.title ?? preparation?.title ?? "録音対象を確認してください"}
    snapshot={snapshot}
    owner={owner}
    locked={locked}
    canStart={canStart}
    canStop={canStop}
    onTitle={actions.setTitle}
    onStart={actions.start}
    onStop={actions.stop}
    onSettings={actions.openSettings}
    onConfirmResult={actions.confirmResult}
    inputs={inputs}
    activeMicrophone={activeMicrophone}
    test={test}
    vocabulary={preparation
      ? <RecordingVocabulary key={preparation.id} items={preparation.meetingVocabulary} locked={vocabularyLocked || preparation.submitted} onChange={actions.setVocabulary} />
      : meeting?.canEditVocabulary
        ? <VocabularyEditor key={meeting.id} meetingId={meeting.id} meetingTitle={meeting.title} locked={locked} />
        : null}
    notices={<>
      <IssueNotice scope="main" type="recording-command" label="録音の操作" message={errors.capture} />
      <IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={errors.confirmation} />
      <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={errors.meeting} />
    </>}
  />;
}
