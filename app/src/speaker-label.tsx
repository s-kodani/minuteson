import type { CSSProperties } from "react";
import { speakerPalette,speakerTone } from "./speaker-colors";

export function SpeakerLabel({meetingId,speakerId,name,unconfirmed=false}: {meetingId:string;speakerId:string;name:string;unconfirmed?:boolean}) {
  const tone=speakerTone(meetingId,speakerId),colors=speakerPalette[tone];
  const style={"--speaker-light-fg":colors[0],"--speaker-light-bg":colors[1],"--speaker-dark-fg":colors[2],"--speaker-dark-bg":colors[3]} as CSSProperties;
  return <span className="speaker-label" data-speaker-id={speakerId} data-speaker-tone={tone} style={style}>{name}{unconfirmed&&<small className="speaker-unset">名前未設定</small>}</span>;
}

// The caller supplies the already validated source snapshot. Never consult
// current analysis or alias a frozen/estimated ID to a merged person here.
export function SpeakerLabels({meetingId,speakerIds,speakers}: {meetingId:string;speakerIds:readonly string[];speakers:readonly {id:string;label:string;displayName:string|null}[]}) {
  return <span className="assigned-speakers" aria-label="発言の話者">{speakerIds.length?speakerIds.map(id=>{
    const speaker=speakers.find(value=>value.id===id);
    return speaker?<SpeakerLabel key={id} meetingId={meetingId} speakerId={id} name={speaker.displayName||speaker.label} unconfirmed={!speaker.displayName}/>:<span key={id}>話者不明</span>;
  }):<span>話者不明</span>}</span>;
}
