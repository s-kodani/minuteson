import { invoke } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { IconButton } from "./action-icon";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChunkPlayer, playbackStartFrame,type AudioPacket, type PlaybackClock, type PlaybackInfo } from "./playback";
import type { DraftUtteranceAudioRequest } from "./transcript-state";
import { registerPlaybackStop } from "./playback-barrier";
import { appShortcut, overlayOpen } from "./keyboard";
import { PlaybackSurfaceControls, usePlaybackSurface } from "./playback-surface";
import { meetingPlaybackLabel, meetingPlaybackStart, validateMeetingPlayback, type MeetingPlaybackStartReference, type MeetingPlaybackMode, type MeetingPlaybackRequest, type MeetingPlaybackRestore } from "./meeting-playback-state";
import { playbackTimeline } from "./playback-follow";
import { PlaybackPreparationPool } from "./playback-preparation";
import type { VoiceSamplePlaybackRequest } from "./voice-state";
import { ToastNotice } from "./toast";
import { validateMarkdownPlayback, type MarkdownIntervalRequest } from "./minutes-audio-state";

export class BrowserAudioClock implements PlaybackClock {
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

export function useTranscriptAudio(meetingId: string, locked: boolean, options: { inline?: boolean; label?: string; scope?: "meeting" | "interval" | "voice"; idleMeetingMode?:MeetingPlaybackMode; active?: boolean } = {}) {
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

  // The shared owner survives tab changes, but never carries audio into another
  // meeting or a screen without playback. Invalidate before publishing a cursor.
  useLayoutEffect(() => { clear(); clock.dispose(); }, [clock, meetingId, options.active]);

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

  const startInner = async (analysisId: string, utteranceId: string, reference?: DraftUtteranceAudioRequest | MeetingPlaybackRequest | MarkdownIntervalRequest | VoiceSamplePlaybackRequest, absoluteStartUs?: number, restore?: MeetingPlaybackRestore, preserveFollow = false) => {
    if (locked) return;
    clear();
    const meeting = reference && "mode" in reference ? reference : null;
    // MeetingPlaybackRequest is a structural subset of the interval request;
    // its extra field is the discriminant for this closed request union.
    const interval = reference && "expectedDurationUs" in reference ? reference as MarkdownIntervalRequest : null;
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
      const split=reference && "expectedDraft" in reference ? reference : null;
      const voiceSample = reference && "voiceSampleId" in reference ? reference : null;
      const info = voiceSample
        ? await nativePreparations.run("prepare_voice_sample_playback", { sampleId: voiceSample.voiceSampleId })
        : interval
        ? await nativePreparations.run("prepare_markdown_interval_playback", { request: interval })
        : meeting
        ? await nativePreparations.run("prepare_meeting_playback", { request: meeting })
        : split
        ? await nativePreparations.run("prepare_draft_utterance_playback",{request:split})
        : await nativePreparations.run("prepare_utterance_playback", { meetingId, analysisId, utteranceId });
      if (generation !== request.current) {
        await closeToken(info.token);
        return;
      }
      token.current = info.token;
      if (!meeting && info.missingRanges?.length) throw new Error("発言音声に会議全体の欠損情報が含まれています");
      const startUs = voiceSample?.expectedStartUs ?? split?.expectedStartUs;
      const endUs = voiceSample?.expectedEndUs ?? split?.expectedEndUs;
      if (reference && !meeting && (info.startUs !== startUs || info.endUs !== endUs)) {
        await closeToken(info.token);
        token.current = null;
        throw new Error(voiceSample ? "確認した登録区間と再生する音声区間が一致しません" : "確認した発言と再生する音声区間が一致しません");
      }
      if (voiceSample && !["mic","system","imported"].includes(info.trackKind)) throw new Error("登録元の音声トラックを確認できません");
      if (interval) validateMarkdownPlayback(info, interval);
      else if (meeting && restore) validateMeetingPlayback(info, meeting.mode, restore);
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

  const startMeeting = (mode: MeetingPlaybackMode, preserve = false, playing = true) => {
    const current = player.snapshot();
    const restore = preserve && current.info ? { frame:current.frame, rate:current.rate, playing:current.playing, endUs:current.info.endUs }
      : preserve && meetingCheckpoint.current ? meetingCheckpoint.current
      : { frame:0, rate:current.rate, playing };
    return start("", `meeting:${mode}`, { meetingId, mode }, undefined, restore, preserve);
  };

  const startMeetingAt = (mode: MeetingPlaybackMode, reference: MeetingPlaybackStartReference, durationUs: number) => {
    if (locked) return Promise.resolve();
    let startUs: number;
    try { startUs = meetingPlaybackStart(reference, durationUs); }
    catch (cause) { setError(String(cause)); return Promise.resolve(); }
    const current = player.snapshot();
    const frame = Number(BigInt(startUs) * 48000n / 1000000n);
    const id = reference.kind === "utterance" ? reference.utteranceId : reference.kind === "draftUtterance" ? reference.request.utteranceId : `markdown:${reference.startUs}:${reference.endUs}`;
    return start("", id, { meetingId, mode, startReference:reference }, startUs,
      { frame, rate:current.rate, playing:true, endUs:durationUs });
  };

  const snapshot = player.snapshot();
  const info = snapshot.info;
  useEffect(()=>{
    if(options.inline||!info||!selected||token.current!==info.token){playbackTimeline.release(owner);return;}
    if(!Number.isSafeInteger(info.startUs)||!Number.isSafeInteger(info.endUs)||!Number.isSafeInteger(snapshot.frame))return;
    const timeUs=Math.min(info.endUs,Number(BigInt(info.startUs)+BigInt(snapshot.frame)*1000000n/BigInt(info.sampleRate)));
    playbackTimeline.publish({owner,session:`${owner}:${followSession.current}`,meetingId,analysisId:originAnalysis.current,utteranceId:options.scope?null:selected,scope:options.scope?"meeting":"utterance",timeUs,playing:snapshot.playing,trackKind:info.trackKind});
  },[info,snapshot.frame,snapshot.playing,selected,owner,meetingId,options.inline,options.scope]);
  const toggle = () => {
    if (locked || preparing || !player.snapshot().info) return;
    if (player.snapshot().playing) player.pause(); else void player.play();
  };
  const meetingScope = options.scope === "meeting";
  const intervalScope = options.scope === "interval";
  const voiceScope = options.scope === "voice";
  const playbackLabel = voiceScope ? "登録した声の再生" : intervalScope ? "議事録の区間再生" : meetingScope ? "会議全体の音声再生" : "発言の音声再生";
  // A visible idle bar does not resume an AudioContext, create a native token
  // or read PCM. Preparation still starts only from an explicit user action.
  // Hide it while another owner presents an utterance/evidence player.
  const idleMeeting = meetingScope && options.idleMeetingMode && !surface?.occupied;
  const playFromBar = () => {
    if (locked || preparing) return;
    if (info) toggle(); else if (options.idleMeetingMode) void startMeeting(options.idleMeetingMode);
  };
  const controls = (selected || idleMeeting) && <div className="audio-player" data-idle={!selected || undefined} role="group" aria-label={playbackLabel} tabIndex={0} aria-keyshortcuts="Space" onKeyDown={event => {
    if (event.target !== event.currentTarget || appShortcut(event.nativeEvent) !== "playback" || overlayOpen(event.currentTarget) || preparing || locked || (!info && !options.idleMeetingMode)) return;
    event.preventDefault(); playFromBar();
  }}>
    <ToastNotice scope={meetingId} type="playback-error" level="error" message={error || snapshot.error ? `音声を再生できません: ${error || snapshot.error}` : ""} />
    {/* Keep the controls mounted; transient status must not add dock rows. */}
    {preparing && <span className="sr-only" role="status">保存音声を検証して再生を準備しています。取消は音声検証の区切りで反映します。</span>}
    <>
      <div className="audio-controls" aria-busy={preparing || snapshot.buffering}>
        <IconButton label={preparing ? "再生準備を取り消す" : snapshot.playing ? "一時停止" : "再生"} icon={preparing ? "close" : snapshot.playing ? "pause" : "play"} onClick={preparing ? clear : playFromBar} disabled={locked} />
        <input type="range" aria-label={voiceScope ? "登録区間内の再生位置" : intervalScope ? "Markdown区間内の再生位置" : meetingScope ? "会議全体の再生位置" : "発言内の再生位置"} title={!info ? "再生後に位置を変更できます" : undefined} min={0} max={info?.totalFrames ?? 1} step={1} value={snapshot.frame} onChange={(event) => player.seek(Number(event.target.value))} disabled={!info || preparing || locked} />
        <IconButton label="先頭へ" icon="restart" onClick={() => { player.pause(); player.seek(0); }} disabled={!info || preparing || locked} />
        <label>速度<select value={snapshot.rate} onChange={(event) => player.setRate(Number(event.target.value))} disabled={preparing || locked}>{[0.75, 1, 1.25, 1.5, 2].map((rate) => <option key={rate} value={rate}>{rate}×</option>)}</select></label>
        <span>{info ? `${durationLabel(snapshot.frame / info.sampleRate)} / ${durationLabel(info.totalFrames / info.sampleRate)}` : "0:00 / --:--"}</span>
        {info && <IconButton className="playback-close" label="再生を閉じる" icon="close" onClick={clear} disabled={locked} />}
      </div>
      <div className="heading-with-info"><span>{voiceScope ? "登録した声の音声区間" : intervalScope ? "議事録の区間" : meetingScope ? meetingPlaybackLabel((info?.trackKind ?? options.idleMeetingMode) as MeetingPlaybackMode) : info?.trackKind === "mic" ? "マイク音声" : info?.trackKind === "system" ? "PC音声" : "保存音声"}</span><InfoPopover label={playbackLabel}>{voiceScope ? "声の特徴量を抽出した登録元トラックの区間だけを再生します。" : intervalScope ? "Markdownに記述された開始・終了時刻の保存音声だけを再生します。マイクとPC音声がある会議では両方を混ぜ、マイクだけの会議ではマイクを再生します。同期精度は未検証です。" : meetingScope ? "選択した種類の保存音声を会議全体の時間軸で再生します。発言や議事録からは開始時刻へ移動し、区間の終了時刻を過ぎても停止操作または音声末尾まで再生を続けます。両方では各音声を半分の音量で混ぜます。実録音の同期やエコー除去の正確さは未検証です。" : "発言の元トラックを再生します。再生範囲は発言に対応した保存音声です。話者推定やトラック間の時刻補正の正確さを保証するものではありません。"}{(error || snapshot.error) && <p role="alert">音声を再生できません: {error || snapshot.error}</p>}</InfoPopover>
      </div>
      {snapshot.buffering && <span className="sr-only" role="status">音声を読み込んでいます。</span>}
    </>
  </div>;
  return { start, startMeeting, startMeetingAt, clear, controls: controls && surface?.target && !options.inline ? <PlaybackSurfaceControls target={surface.target}>{controls}</PlaybackSurfaceControls> : controls,
    sharedSurface:!!surface, selected, preparing, toggle, available: !!info, playing: snapshot.playing,
    atEnd:!!info && snapshot.frame >= info.totalFrames };
}
