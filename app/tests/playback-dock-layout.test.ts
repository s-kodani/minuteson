import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source = (name:string) => readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");

test("initial meeting bar is present without mounting a second playback entry or native preparation", () => {
  const meeting=source("meeting-audio.tsx"), player=source("transcript-audio.tsx");
  assert.match(meeting,/idleMeetingMode:mode/);
  assert.doesNotMatch(meeting,/meeting-audio-start|会議全体を再生/);
  assert.match(meeting,/audio.clear\(\); setModes\(\[\]\); setSummaryError\(""\)/);
  assert.match(meeting,/get_meeting_audio_summary/);
  assert.match(meeting,/preferredMeetingPlaybackMode\(summary.modes\)/);
  assert.match(player,/const idleMeeting = meetingScope && options.idleMeetingMode && !surface\?\.occupied/);
  assert.match(player,/\(selected \|\| idleMeeting\) && <div className="audio-player"/);
  assert.match(player,/if \(locked \|\| preparing\) return;\s*if \(info\) toggle\(\); else if \(options.idleMeetingMode\) void startMeeting\(options.idleMeetingMode\)/);
  assert.match(player,/disabled=\{!info \|\| preparing \|\| locked\}/);
  assert.match(player,/"0:00 \/ --:--"/);
});

test("the shared surface hides idle controls while any same-meeting audio owner is active", () => {
  const surface=source("playback-surface.tsx");
  assert.match(surface,/owner\?\.meetingId===meetingId \? owner : null/);
  assert.match(surface,/current\?\.label \|\| "保存音声"/);
  assert.doesNotMatch(surface,/current\?\.label \|\| "発言区間の音声"/);
  assert.match(surface,/occupied:!!current/);
  assert.match(surface,/release:id=>setOwner\(current=>current\?\.id===id \? null : current\)/);
  assert.match(source("transcript-audio.tsx"),/window.addEventListener\("mon-audio-start", otherStarted\)/);
});

test("dock settings align with the first controls row rather than the whole multi-line player", () => {
  const css=source("prototype-alignment.css");
  assert.match(css,/\.meeting-playback-dock \{ display:grid; grid-template-columns:minmax\(0,1fr\) auto; align-items:start;/);
  assert.match(css,/\.meeting-playback-dock \.audio-player \{ display:grid; grid-template-columns:minmax\(0,1fr\) auto; align-items:center;/);
  assert.match(css,/\.meeting-playback-dock \.meeting-audio-actions \{[^}]*align-self:start;/);
  assert.match(css,/\.meeting-audio-settings \{ width:auto; \}/);
  assert.match(css,/:root \.meeting-playback-dock \.audio-controls>button\.playback-close \{ display:none; \}/);
  assert.doesNotMatch(css,/playback-alignment-note/);
});

test("playback status never adds a visible dock row, and preparation retains a same-size cancel control", () => {
  const player=source("transcript-audio.tsx");
  assert.doesNotMatch(player,/playback-alignment|同期未検証：トラック間の同期精度は未検証です/);
  assert.match(player,/<ToastNotice scope=\{meetingId\} type="playback-error"/);
  assert.doesNotMatch(player,/playback-alignment-note|\(info \|\| options.idleMeetingMode\) && !preparing/);
  assert.match(player,/preparing && <span className="sr-only" role="status"/);
  assert.match(player,/snapshot.buffering && <span className="sr-only" role="status"/);
  assert.match(player,/label=\{preparing \? "再生準備を取り消す"/);
  assert.match(player,/onClick=\{preparing \? clear : playFromBar\} disabled=\{locked\}/);
  assert.doesNotMatch(player,/PlaybackMissing|音声欠損を確認|<OptionsModal/);
  assert.match(player,/max=\{info\?\.totalFrames \?\? 1\} step=\{1\}/);
});

test("meeting playback retains its paused start contract without an extra settings modal", () => {
  const meeting=source("meeting-audio.tsx"), player=source("transcript-audio.tsx");
  assert.doesNotMatch(meeting,/OptionsModal|再生設定/);
  assert.match(player,/const startMeeting = \(mode: MeetingPlaybackMode, preserve = false, playing = true\)/);
  assert.match(player,/: \{ frame:0, rate:current.rate, playing \}/);
  assert.match(player,/if \(!meeting \|\| restore\?\.playing !== false\) await player.play\(\)/);
});
