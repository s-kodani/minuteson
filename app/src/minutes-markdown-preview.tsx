import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ActionIcon } from "./action-icon";
import { markdownAudioInterval, playableMarkdownInterval, type MarkdownAudioInterval } from "./minutes-markdown-state";

export function MinutesMarkdownPreview({ text, durationUs, locked = false, onPlay }: {
  text: string; durationUs: number | null; locked?: boolean;
  onPlay?: (interval: MarkdownAudioInterval) => void;
}) {
  return <div className="minutes-markdown-preview">
    <Markdown remarkPlugins={[remarkGfm]} skipHtml
      urlTransform={url => markdownAudioInterval(url) ? url : ""}
      components={{
        // No remote/local image fetches, HTML plugin, executable URL, or native
        // external navigation. Ordinary link text remains visible as text.
        img: ({ alt }) => <span className="markdown-image-label">{alt ?? ""}</span>,
        a: ({ href, children }) => {
          const interval = href ? playableMarkdownInterval(href, durationUs) : null;
          return interval && onPlay ? <button type="button" className="markdown-audio-link" disabled={locked}
            title="この区間を再生" aria-label={`この区間を再生: ${href!.slice(6).replace("-", "–")}`}
            onClick={() => onPlay(interval)}><ActionIcon name="play" /><span>{children}</span></button> : <span>{children}</span>;
        },
        table: ({ children }) => <div className="markdown-table-scroll"><table>{children}</table></div>,
      }}>{text}</Markdown>
  </div>;
}
