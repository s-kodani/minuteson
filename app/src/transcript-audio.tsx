import { invoke } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { useEffect, useRef, useState } from "react";
import { ChunkPlayer, playbackStartFrame,type AudioPacket, type PlaybackClock, type PlaybackInfo } from "./playback";
import type { MinutesEvidenceRequest, MinutesTranscriptAudioRequest } from "./minutes-state";
import type { DraftUtteranceAudioRequest } from "./transcript-state";
import { registerPlaybackStop } from "./playback-barrier";
import { appShortcut, overlayOpen } from "./keyboard";
import { PlaybackSurfaceControls, usePlaybackSurface } from "./playback-surface";
import { meetingPlaybackLabel, validateMeetingPlayback, type MeetingPlaybackMode, type MeetingPlaybackRequest, type MeetingPlaybackRestore } from "./meeting-playback-state";
import { playbackTimeline } from "./playback-follow";
import { PlaybackPreparationPool } from "./playback-preparation";

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

export function useTranscriptAudio(meetingId: string, locked: boolean, options: { inline?: boolean; label?: string; scope?: "meeting" } = {}) {
  const surface=usePlaybackSurface();
  const [, refresh] = useState(0);
  const [clock] = useState(() => new BrowserAudioClock());
  const [player] = useState(() => new ChunkPlayer(clock, (token, offsetFrames) => invoke<ArrayBuffer | number[]>("read_playback_chunk", { token, offsetFrames }), () => refresh((value) => value + 1)));
  const [selected, setSelected] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState("");
  const request = useRef(0);
  const token = useRef<string | null>(null);
  const meetingCheckpoint = useRef<MeetingPlaybackRestore | null>(null);
  const followSession = useRef(0),originAnalysis = useRef<string|null>(null);
  const [owner] = useState(() => crypto.randomUUID());
  const [nativePreparations]=useState(()=>new PlaybackPreparationPool({
    register:preparationId=>invoke("register_playback_preparation",{preparationId}),
    prepare:(command,args)=>invoke<PlaybackInfo>(command,args),
    cancel:preparationId=>invoke("cancel_playback_preparation",{preparationId}),
    acknowledge:(preparationId,playbackToken)=>invoke("finish_playback_preparation",{preparationId,playbackToken}),
    close:token=>invoke("close_playback",{token}),
  }));
  const preparations = useRef(new Set<Promise<void>>());
  const closing = useRef(new Map<string, { job: Promise<void>; active: boolean }>());
  const closeToken = (id: string): Promise<void> => {
    const previous = closing.current.get(id);
    if (previous?.active) return previous.job;
    const entry = { job: Promise.resolve(), active: true };
    entry.job = invoke<void>("close_playback", { token: id }).then(() => { closing.current.delete(id); }, cause => { entry.active = false; throw cause; });
    closing.current.set(id, entry);
    return entry.job;
  };

  const clear = () => {
    nativePreparations.cancelAll();
    playbackTimeline.release(owner);
    surface?.release(owner);
    request.current++;
    player.clear();
    if (token.current) void closeToken(token.current).catch(() => {});
    token.current = null;
    meetingCheckpoint.current = null;
    setSelected(null);
    setPreparing(false);
    setError("");
  };

  useEffect(() => {
    let detached = false;
    const stop = async () => {
      if (detached) {
        nativePreparations.cancelAll();
        request.current++; player.clear();
        if (token.current) void closeToken(token.current).catch(() => {});
        token.current = null;
      } else clear();
      await Promise.all(preparations.current);
      await nativePreparations.stop();
      await Promise.all(Array.from(closing.current.keys(), closeToken));
      if (detached) unregister();
    };
    const unregister = registerPlaybackStop(stop);
    const otherStarted = (event: Event) => { if ((event as CustomEvent<string>).detail !== owner) clear(); };
    window.addEventListener("mon-audio-start", otherStarted);
    const timer = window.setInterval(() => { if (player.snapshot().playing) refresh((value) => value + 1); }, 100);
    return () => {
      // Keep failed/in-flight native releases in the global barrier even after
      // this view disappears. Deletion must not mistake unmount for release.
      detached = true;
      surface?.release(owner);
      playbackTimeline.release(owner);
      window.clearInterval(timer);
      window.removeEventListener("mon-audio-start", otherStarted);
      request.current++;
      player.clear();
      void stop().catch(() => {});
      clock.dispose();
    };
  }, [clock, player, owner,nativePreparations]);

  const startInner = async (analysisId: string, utteranceId: string, reference?: MinutesEvidenceRequest | MinutesTranscriptAudioRequest | DraftUtteranceAudioRequest | MeetingPlaybackRequest, absoluteStartUs?: number, restore?: MeetingPlaybackRestore, preserveFollow = false) => {
    if (locked) return;
    clear();
    const meeting = reference && "mode" in reference ? reference : null;
    if(!preserveFollow)followSession.current++;
    originAnalysis.current=meeting?null:analysisId;
    if (meeting) meetingCheckpoint.current = restore ?? null;
    window.dispatchEvent(new CustomEvent("mon-audio-start", { detail: owner }));
    if (!options.inline) surface?.claim({ id:owner,meetingId,label:options.label ?? "文字起こしの発言音声" });
    const generation = request.current;
    setSelected(utteranceId);
    setPreparing(true);
    try {
      // Resume in the user's click, before hashing or IPC awaits lose activation.
      await clock.resume();
      if (generation !== request.current) return;
      const transcript = reference && "reference" in reference ? reference : null;
      const split=reference && "expectedDraft" in reference ? reference : null;
      const info = meeting
        ? await nativePreparations.run("prepare_meeting_playback", { request: meeting })
        : split
        ? await nativePreparations.run("prepare_draft_utterance_playback",{request:split})
        : transcript
        ? await nativePreparations.run("prepare_minutes_transcript_playback", { request: transcript })
        : reference
        ? await nativePreparations.run("prepare_minutes_evidence_playback", { request: reference })
        : await nativePreparations.run("prepare_utterance_playback", { meetingId, analysisId, utteranceId });
      if (generation !== request.current) {
        await closeToken(info.token);
        return;
      }
      token.current = info.token;
      const startUs = split?.expectedStartUs ?? transcript?.expectedStartUs ?? (reference && "expectedEvidence" in reference ? reference.expectedEvidence.startUs : undefined);
      const endUs = split?.expectedEndUs ?? transcript?.expectedEndUs ?? (reference && "expectedEvidence" in reference ? reference.expectedEvidence.endUs : undefined);
      if (reference && !meeting && (info.startUs !== startUs || info.endUs !== endUs)) {
        await closeToken(info.token);
        token.current = null;
        throw new Error("確認した発言と再生する音声区間が一致しません");
      }
      if (meeting && restore) validateMeetingPlayback(info, meeting.mode, restore);
      const startFrame=absoluteStartUs===undefined?null:playbackStartFrame(info,absoluteStartUs);
      player.load(info);
      if (meeting && restore) { player.setRate(restore.rate); player.seek(restore.frame); }
      if(startFrame!==null)player.seek(startFrame);
      if (!meeting || restore?.playing !== false) await player.play();
    } catch (cause) {
      if (generation === request.current) {
        player.clear();
        if(token.current)void closeToken(token.current).catch(()=>{});
        token.current=null;
        meetingCheckpoint.current=null;
        setError(String(cause));
      }
    } finally {
      if (generation === request.current) setPreparing(false);
    }
  };

  const start = (...args: Parameters<typeof startInner>) => {
    const job = startInner(...args);
    preparations.current.add(job);
    void job.then(() => preparations.current.delete(job), () => preparations.current.delete(job));
    return job;
  };

  const startMeeting = (mode: MeetingPlaybackMode, preserve = false) => {
    const current = player.snapshot();
    const restore = preserve && current.info ? { frame:current.frame, rate:current.rate, playing:current.playing, endUs:current.info.endUs }
      : preserve && meetingCheckpoint.current ? meetingCheckpoint.current
      : { frame:0, rate:current.rate, playing:true };
    return start("", `meeting:${mode}`, { meetingId, mode }, undefined, restore, preserve);
  };

  const snapshot = player.snapshot();
  const info = snapshot.info;
  useEffect(()=>{
    if(options.inline||!info||!selected||token.current!==info.token){playbackTimeline.release(owner);return;}
    if(!Number.isSafeInteger(info.startUs)||!Number.isSafeInteger(info.endUs)||!Number.isSafeInteger(snapshot.frame))return;
    const timeUs=Math.min(info.endUs,Number(BigInt(info.startUs)+BigInt(snapshot.frame)*1000000n/BigInt(info.sampleRate)));
    playbackTimeline.publish({owner,session:`${owner}:${followSession.current}`,meetingId,analysisId:originAnalysis.current,utteranceId:options.scope==="meeting"?null:selected,scope:options.scope??"utterance",timeUs,playing:snapshot.playing,trackKind:info.trackKind});
  },[info,snapshot.frame,snapshot.playing,selected,owner,meetingId,options.inline,options.scope]);
  const toggle = () => {
    if (locked || preparing || !player.snapshot().info) return;
    if (player.snapshot().playing) player.pause(); else void player.play();
  };
  const meetingScope = options.scope === "meeting";
  const controls = selected && <div className="audio-player" role="group" aria-label={meetingScope ? "会議全体の音声再生" : "発言の音声再生"} tabIndex={0} aria-keyshortcuts="Space" onKeyDown={event => {
    if (event.target !== event.currentTarget || appShortcut(event.nativeEvent) !== "playback" || overlayOpen(event.currentTarget) || preparing || locked || !info) return;
    event.preventDefault(); toggle();
  }}>
    {preparing && <p role="status">保存音声を検証して再生を準備しています。</p>}
    {preparing && <button type="button" className="small-secondary" disabled={locked} onClick={clear}>再生準備を取り消す</button>}
    {preparing && <p>取消は音声検証の区切りで反映します。停止完了と準備済み音声の解放まで待ちます。</p>}
    {info && <>
      <div className="audio-controls">
        <button type="button" className="small-secondary" onClick={toggle} disabled={preparing || locked}>{snapshot.playing ? "一時停止" : "再生"}</button>
        <button type="button" className="small-secondary" onClick={() => { player.pause(); player.seek(0); }} disabled={locked}>先頭へ</button>
        <label>速度<select value={snapshot.rate} onChange={(event) => player.setRate(Number(event.target.value))} disabled={locked}>{[0.75, 1, 1.25, 1.5, 2].map((rate) => <option key={rate} value={rate}>{rate}×</option>)}</select></label>
        <span>{durationLabel(snapshot.frame / info.sampleRate)} / {durationLabel(info.totalFrames / info.sampleRate)}</span>
        <button type="button" className="small-secondary" onClick={clear} disabled={locked}>再生を閉じる</button>
      </div>
      <input type="range" aria-label={meetingScope ? "会議全体の再生位置" : "発言内の再生位置"} min={0} max={info.totalFrames} step={Math.max(1, Math.floor(info.sampleRate / 10))} value={snapshot.frame} onChange={(event) => player.seek(Number(event.target.value))} disabled={preparing || locked} />
      <div className="heading-with-info"><span>{meetingScope ? meetingPlaybackLabel(info.trackKind as MeetingPlaybackMode) : info.trackKind === "mic" ? "マイク音声" : info.trackKind === "system" ? "PC音声" : "保存音声"}</span><InfoPopover label={meetingScope ? "会議全体の音声再生" : "発言の音声再生"}>{meetingScope ? "選択した種類の保存音声を会議全体の時間軸で再生します。両方では各音声を半分の音量で混ぜます。実録音の同期やエコー除去の正確さは未検証です。" : "発言の元トラックを再生します。再生範囲は発言に対応した保存音声です。話者推定やトラック間の時刻補正の正確さを保証するものではありません。"}</InfoPopover></div>
      {info.alignment === "unverified" && <p>会議時間軸との同期精度は未検証です。</p>}
      {snapshot.buffering && <p role="status">音声を読み込んでいます。</p>}
    </>}
    {(error || snapshot.error) && <p className="action-error" role="alert">音声を再生できません: {error || snapshot.error}</p>}
  </div>;
  return { start, startMeeting, clear, controls: controls && surface?.target && !options.inline ? <PlaybackSurfaceControls target={surface.target}>{controls}</PlaybackSurfaceControls> : controls,
    sharedSurface:!!surface, selected, preparing, toggle, available: !!info,playing: snapshot.playing };
}
