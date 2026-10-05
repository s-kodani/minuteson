// Development-only mount of the production hook. No microphone or real audio.
import { createRoot } from "react-dom/client";
import { useTranscriptAudio } from "../src/transcript-audio";

function PlaybackHarness({ meetingId }: { meetingId: string }) {
  const audio = useTranscriptAudio(meetingId, false);
  return <section aria-label="模擬音声準備"><button type="button" onClick={() => void audio.start("fixture-analysis", "fixture-utterance")}>模擬区間の再生準備</button>{audio.controls}</section>;
}

export function mountPlaybackHarness(meetingId: string) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); root.render(<PlaybackHarness meetingId={meetingId} />);
  return () => { root.unmount(); host.remove(); };
}
