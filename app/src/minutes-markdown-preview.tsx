import { createContext, memo, useContext, type ComponentPropsWithoutRef } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ActionIcon } from "./action-icon";
import { markdownAudioInterval, playableMarkdownInterval, type MarkdownAudioInterval } from "./minutes-markdown-state";

type AudioLinks = { durationUs: number | null; locked: boolean; onPlay?: (interval: MarkdownAudioInterval) => void };
const AudioLinksContext = createContext<AudioLinks>({ durationUs: null, locked: false });
function AudioLink({ href, children }: ComponentPropsWithoutRef<"a">) {
  const { durationUs, locked, onPlay } = useContext(AudioLinksContext);
  const interval = href ? playableMarkdownInterval(href, durationUs) : null;
  return interval && onPlay ? <button type="button" className="markdown-audio-link" disabled={locked}
    title="この区間の開始時刻から再生" aria-label={`この区間の開始時刻から再生: ${href!.slice(6).replace("-", "–")}`}
    onClick={() => onPlay(interval)}><ActionIcon name="play" /><span>{children}</span></button> : <span>{children}</span>;
}
// Stable component identities preserve the actual button from pointer-down to
// click while playback clocks update. The immutable document is parsed once.
const components: Components = {
  img: ({ alt }) => <span className="markdown-image-label">{alt ?? ""}</span>,
  a: AudioLink,
  table: ({ children }) => <div className="markdown-table-scroll"><table>{children}</table></div>,
};
const plugins = [remarkGfm];
const audioUrl = (url: string) => markdownAudioInterval(url) ? url : "";
const DocumentBody = memo(function DocumentBody({ text }: { text: string }) {
  // Images, HTML and non-audio links cannot fetch or navigate externally.
  return <Markdown remarkPlugins={plugins} skipHtml urlTransform={audioUrl} components={components}>{text}</Markdown>;
});
export function MinutesMarkdownPreview({ text, durationUs, locked = false, onPlay }: { text: string } & Omit<AudioLinks, "locked"> & { locked?: boolean }) {
  return <AudioLinksContext.Provider value={{ durationUs, locked, onPlay }}>
    <div className="minutes-markdown-preview"><DocumentBody text={text} /></div>
  </AudioLinksContext.Provider>;
}
