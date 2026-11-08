import { memo, type RefObject } from "react";
import { IconButton } from "./action-icon";
import { SpeakerLabels } from "./speaker-label";
import { IdentificationBadge } from "./speaker-identification";
import { appShortcut, overlayOpen } from "./keyboard";
import { shouldOpenTranscriptBody, shouldOpenTranscriptBodyFromKey } from "./transcript-body-activation";
import type { Speaker, Utterance } from "./transcript-state";

export function timeLabel(microseconds: number): string {
  const seconds = Math.floor(microseconds / 1_000_000);
  return [Math.floor(seconds / 3600), Math.floor(seconds % 3600 / 60), seconds % 60].map(value => String(value).padStart(2, "0")).join(":");
}

// The editor owns the current lock, audio owner and draft. Memoized rows call
// these live handlers rather than capturing an old revision or playback token.
export interface TranscriptRowActions {
  play: (row: Utterance, toggle: boolean) => void;
  openText: (row: Utterance) => void;
  changeText: (id: string, text: string) => void;
  saveText: (id: string) => void;
  cancelText: (id: string) => void;
  assign: (row: Utterance, opener: HTMLElement) => void;
}
type SharedProps = {
  colorKeys?: Readonly<Record<string,string>>;
  meetingId: string; speakers: readonly Speaker[]; canPlay: boolean;
  saveFailed: boolean; dirty: boolean; actions: RefObject<TranscriptRowActions>;
};

const TranscriptRow = memo(function TranscriptRow({ utterance, text, playing, meetingId, speakers, colorKeys, canPlay, saveFailed, dirty, actions }: SharedProps & {
  utterance: Utterance; text: string | undefined; playing: boolean;
}) {
  const label = timeLabel(utterance.startUs);
  return <article data-utterance-id={utterance.id} className={`utterance-row${playing ? " utterance-playing" : ""}`}
    aria-current={playing ? "true" : undefined} tabIndex={0} aria-label={`${label}の発言行`} aria-keyshortcuts="Space"
    onKeyDown={event => {
      if (event.target !== event.currentTarget || !canPlay || appShortcut(event.nativeEvent) !== "playback" || overlayOpen()) return;
      event.preventDefault(); actions.current.play(utterance, true);
    }}>
    <div className="utterance-meta"><SpeakerLabels meetingId={meetingId} speakerIds={utterance.speakerIds} speakers={speakers} colorKeys={colorKeys} />
      <button type="button" className="utterance-stamp" aria-label="音声を再生" title={label + "–" + timeLabel(utterance.endUs)}
        disabled={!canPlay} onClick={() => actions.current.play(utterance, false)}><time>{label}</time></button></div>
    <IdentificationBadge value={utterance.speakerIdentification}/>
    {text !== undefined ? <div className="utterance-text-edit">
      <textarea autoFocus aria-label={label + "の発言"} rows={2} maxLength={20000} value={text}
        onChange={event => actions.current.changeText(utterance.id, event.target.value)} onKeyDown={event => {
          if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || event.repeat) return;
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); actions.current.saveText(utterance.id); }
          if (event.key === "Escape") { event.preventDefault(); actions.current.cancelText(utterance.id); }
        }} />
      <div className="transcript-actions"><button type="button" disabled={saveFailed} onClick={() => actions.current.saveText(utterance.id)}>保存</button>
        <button type="button" className="small-secondary" disabled={saveFailed || dirty} onClick={() => actions.current.cancelText(utterance.id)}>キャンセル</button></div>
    </div> : <div className="utterance-text"><span role="button" className="transcript-body-edit" aria-label={label + "の発言を編集"} tabIndex={0}
      onKeyDown={event => { if (shouldOpenTranscriptBodyFromKey(event.nativeEvent)) { event.preventDefault(); actions.current.openText(utterance); } }}
      onClick={event => { const body = event.currentTarget; if (shouldOpenTranscriptBody(event.detail, body.ownerDocument.getSelection(), node => body.contains(node))) actions.current.openText(utterance); }}>{utterance.effectiveText}</span></div>}
    <div className="utterance-edit-tools options-modal">
      <IconButton className="options-modal-trigger" label={label + "の話者を変更"} icon="more" aria-haspopup="dialog" disabled={saveFailed}
        onClick={event => actions.current.assign(utterance, event.currentTarget)} />
    </div>
  </article>;
});

// Visibility and transient locks belong to the surrounding panel/fieldset, not
// every row. Polling, navigation and playback clock ticks can reuse this tree.
export const TranscriptRows = memo(function TranscriptRows({ rows, texts, playingIds, ...shared }: SharedProps & {
  rows: readonly Utterance[]; texts: Readonly<Record<string, string>>; playingIds: ReadonlySet<string>;
}) {
  return rows.map(utterance => <TranscriptRow key={utterance.id} {...shared} utterance={utterance}
    text={Object.hasOwn(texts, utterance.id) ? texts[utterance.id] : undefined} playing={playingIds.has(utterance.id)} />);
});
