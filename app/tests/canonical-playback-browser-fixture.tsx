// Exercise the production hook with native-shaped 16 kHz replies and silent PCM.
import { createRoot } from "react-dom/client";
import { useTranscriptAudio } from "../src/transcript-audio";
import { ToastHost } from "../src/toast";

const fixture = { calls: [] as {command:string; args:any}[], sessions:new Map<string,any>(), rate:16000 };
(window as any).canonicalPlaybackFixture=fixture;
(window as any).__TAURI_INTERNALS__={invoke:async(command:string,args:any={})=>{
  fixture.calls.push({command,args:structuredClone(args)});
  if(command.startsWith("prepare_")) {
    const r=args.request, interval=command==="prepare_markdown_interval_playback";
    const startUs=interval?r.startUs:0,endUs=interval?r.endUs:1000000;
    const sampleRate=fixture.rate;
    const info={token:crypto.randomUUID(),sampleRate,channels:1,startUs,endUs,
      totalFrames:Math.ceil(endUs*sampleRate/1e6)-Math.ceil(startUs*sampleRate/1e6),
      trackKind:"mixed",alignment:"unverified",missingRanges:[]};
    fixture.sessions.set(info.token,info);return info;
  }
  if(command==="read_playback_chunk") {
    const info=fixture.sessions.get(args.token), frames=Math.min(65536,info.totalFrames-args.offsetFrames);
    if(frames<=0)throw Error("read past EOF");
    const bytes=new ArrayBuffer(12+frames*4),view=new DataView(bytes);
    view.setUint32(0,info.sampleRate,true);view.setUint32(4,1,true);view.setUint32(8,frames,true);return bytes;
  }
  if(command==="close_playback")fixture.sessions.delete(args.token);
}};

function Fixture() {
  const audio=useTranscriptAudio("synthetic",false,{scope:"meeting",idleMeetingMode:"mixed"});
  const startUs=750001,endUs=900000;
  return <><button onClick={()=>void audio.startMeeting("mixed")}>全体</button>
    <button onClick={()=>void audio.startMeetingAt("mixed",{kind:"utterance",analysisId:"a",utteranceId:"u",expectedContentHash:"hash",startUs,endUs},1000000)}>発言</button>
    <button onClick={()=>void audio.startMeetingAt("mixed",{kind:"draftUtterance",request:{meetingId:"synthetic",analysisId:"a",utteranceId:"d",expectedContentHash:"hash",expectedDraft:{id:"draft",lockVersion:1},expectedStartUs:startUs,expectedEndUs:endUs}},1000000)}>分割発言</button>
    <button onClick={()=>void audio.startMeetingAt("mixed",{kind:"interval",startUs,endUs,expectedDurationUs:1000000},1000000)}>議事録の開始位置</button>
    <button onClick={()=>void audio.start("","interval",{meetingId:"synthetic",mode:"mixed",startUs:250001,endUs:300000,expectedDurationUs:1000000})}>区間だけ</button>
    {audio.controls}<ToastHost /></>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
