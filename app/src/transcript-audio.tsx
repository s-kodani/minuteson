import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { ChunkPlayer, playbackStartFrame,type AudioPacket, type PlaybackClock, type PlaybackInfo } from "./playback";
import type { MinutesEvidenceRequest, MinutesTranscriptAudioRequest } from "./minutes-state";
import type { DraftUtteranceAudioRequest } from "./transcript-state";

class BrowserAudioClock implements PlaybackClock {
  private context: AudioContext | null = null;
  now() { return this.context?.currentTime ?? 0; }
  async resume() {
    this.context ??= new AudioContext();
    await this.context.resume();
  }
  schedule(packet: AudioPacket, at: number, rate: number, ended: () => void) {
    const context = this.context!;
    const buffer = context.createBuffer(packet.channels.length, packet.frames, packet.sampleRate);
    packet.channels.forEach((samples, channel) => buffer.getChannelData(channel).set(samples));
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.playbackRate.value = rate;
    source.connect(context.destination);
    source.onended = () => { source.disconnect(); ended(); };
    source.start(at);
    return () => { source.onended = null; source.stop(); source.disconnect(); };
  }
  dispose() { void this.context?.close(); this.context = null; }
}

function durationLabel(seconds: number) {
  const whole = Math.floor(seconds);
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}

export function useTranscriptAudio(meetingId: string, locked: boolean) {
  const [, refresh] = useState(0);
  const [clock] = useState(() => new BrowserAudioClock());
  const [player] = useState(() => new ChunkPlayer(clock, (token, offsetFrames) => invoke<ArrayBuffer | number[]>("read_playback_chunk", { token, offsetFrames }), () => refresh((value) => value + 1)));
  const [selected, setSelected] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState("");
  const request = useRef(0);
  const token = useRef<string | null>(null);
  const [owner] = useState(() => crypto.randomUUID());

  const clear = () => {
    request.current++;
    player.clear();
    if (token.current) void invoke("close_playback", { token: token.current }).catch(() => {});
    token.current = null;
    setSelected(null);
    setPreparing(false);
    setError("");
  };

  useEffect(() => {
    const otherStarted = (event: Event) => { if ((event as CustomEvent<string>).detail !== owner) clear(); };
    window.addEventListener("mon-audio-start", otherStarted);
    const timer = window.setInterval(() => { if (player.snapshot().playing) refresh((value) => value + 1); }, 100);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("mon-audio-start", otherStarted);
      request.current++;
      player.clear();
      if (token.current) void invoke("close_playback", { token: token.current }).catch(() => {});
      clock.dispose();
    };
  }, [clock, player, owner]);

  const start = async (analysisId: string, utteranceId: string, reference?: MinutesEvidenceRequest | MinutesTranscriptAudioRequest | DraftUtteranceAudioRequest, absoluteStartUs?: number) => {
    if (locked) return;
    clear();
    window.dispatchEvent(new CustomEvent("mon-audio-start", { detail: owner }));
    const generation = request.current;
    setSelected(utteranceId);
    setPreparing(true);
    try {
      // Resume in the user's click, before hashing or IPC awaits lose activation.
      await clock.resume();
      if (generation !== request.current) return;
      const transcript = reference && "reference" in reference ? reference : null;
      const split=reference && "expectedDraft" in reference ? reference : null;
      const info = split
        ? await invoke<PlaybackInfo>("prepare_draft_utterance_playback",{request:split})
        : transcript
        ? await invoke<PlaybackInfo>("prepare_minutes_transcript_playback", { request: transcript })
        : reference
        ? await invoke<PlaybackInfo>("prepare_minutes_evidence_playback", { request: reference })
        : await invoke<PlaybackInfo>("prepare_utterance_playback", { meetingId, analysisId, utteranceId });
      if (generation !== request.current) {
        void invoke("close_playback", { token: info.token }).catch(() => {});
        return;
      }
      token.current = info.token;
      const startUs = split?.expectedStartUs ?? transcript?.expectedStartUs ?? (reference && "expectedEvidence" in reference ? reference.expectedEvidence.startUs : undefined);
      const endUs = split?.expectedEndUs ?? transcript?.expectedEndUs ?? (reference && "expectedEvidence" in reference ? reference.expectedEvidence.endUs : undefined);
      if (reference && (info.startUs !== startUs || info.endUs !== endUs)) {
        void invoke("close_playback", { token: info.token }).catch(() => {});
        token.current = null;
        throw new Error("確認した発言と再生する音声区間が一致しません");
      }
      const startFrame=absoluteStartUs===undefined?null:playbackStartFrame(info,absoluteStartUs);
      player.load(info);
      if(startFrame!==null)player.seek(startFrame);
      await player.play();
    } catch (cause) {
      if (generation === request.current) {
        player.clear();
        if(token.current)void invoke("close_playback",{token:token.current}).catch(()=>{});
        token.current=null;
        setError(String(cause));
      }
    } finally {
      if (generation === request.current) setPreparing(false);
    }
  };

  const snapshot = player.snapshot();
  const info = snapshot.info;
  const controls = selected && <div className="audio-player" aria-label="発言の音声再生">
    {preparing && <p role="status">保存音声を検証して再生を準備しています。</p>}
    {info && <>
      <div className="audio-controls">
        <button type="button" className="small-secondary" onClick={() => { if (snapshot.playing) player.pause(); else void player.play(); }} disabled={preparing || locked}>{snapshot.playing ? "一時停止" : "再生"}</button>
        <button type="button" className="small-secondary" onClick={() => { player.pause(); player.seek(0); }} disabled={locked}>先頭へ</button>
        <label>速度<select value={snapshot.rate} onChange={(event) => player.setRate(Number(event.target.value))} disabled={locked}>{[0.75, 1, 1.25, 1.5, 2].map((rate) => <option key={rate} value={rate}>{rate}×</option>)}</select></label>
        <span>{durationLabel(snapshot.frame / info.sampleRate)} / {durationLabel(info.totalFrames / info.sampleRate)}</span>
        <button type="button" className="small-secondary" onClick={clear} disabled={locked}>再生を閉じる</button>
      </div>
      <input type="range" aria-label="発言内の再生位置" min={0} max={info.totalFrames} step={Math.max(1, Math.floor(info.sampleRate / 10))} value={snapshot.frame} onChange={(event) => player.seek(Number(event.target.value))} disabled={preparing || locked} />
      <p>{info.trackKind === "mic" ? "マイク音声" : info.trackKind === "system" ? "PC音声" : "保存音声"} · 発言の元トラックを再生します。{info.alignment === "unverified" && "会議時間軸との同期精度は未検証です。"}</p>
      {snapshot.buffering && <p role="status">音声を読み込んでいます。</p>}
    </>}
    {(error || snapshot.error) && <p className="action-error" role="alert">音声を再生できません: {error || snapshot.error}</p>}
  </div>;
  return { start, clear, controls, selected, preparing };
}
