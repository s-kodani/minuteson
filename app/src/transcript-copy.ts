import type { Analysis } from "./transcript-state.ts";

/** Copies the complete projected transcript, independently of UI filters. */
export function transcriptCopyText(view: Analysis): string {
  const names = new Map(view.speakers.map(speaker => [speaker.id, speaker.displayName || speaker.label]));
  return view.utterances.map(row => {
    const seconds = Math.floor(row.startUs / 1_000_000);
    const time = [Math.floor(seconds / 3600), Math.floor(seconds % 3600 / 60), seconds % 60]
      .map(value => String(value).padStart(2, "0")).join(":");
    const speakers = row.speakerIds.map(id => names.get(id) || "話者不明").join("、") || "話者不明";
    return `[${time}] ${speakers}：${row.effectiveText}`;
  }).join("\n");
}

/** Start the native write in the click gesture, even when edits need saving.
 * WebKit otherwise rejects writeText after an asynchronous save. */
export async function writeTranscriptClipboard(prepare: () => Promise<string>): Promise<void> {
  const text = prepare();
  try {
    const write = typeof ClipboardItem !== "undefined" && typeof navigator.clipboard.write === "function"
      ? navigator.clipboard.write([new ClipboardItem({ "text/plain": text.then(value => new Blob([value], { type: "text/plain" })) })])
      : text.then(value => navigator.clipboard.writeText(value));
    await Promise.all([text, write]);
  } catch (cause) {
    await text.catch(() => {});
    throw cause;
  }
}
