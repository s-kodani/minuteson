import type { CSSProperties } from "react";
import { speakerPalette,speakerTone } from "./speaker-colors";

export function SpeakerLabel({meetingId,speakerId,name,unconfirmed=false}: {meetingId:string;speakerId:string;name:string;unconfirmed?:boolean}) {
  const tone=speakerTone(meetingId,speakerId),colors=speakerPalette[tone];
  const style={"--speaker-light-fg":colors[0],"--speaker-light-bg":colors[1],"--speaker-dark-fg":colors[2],"--speaker-dark-bg":colors[3]} as CSSProperties;
  return <span className="speaker-label" data-speaker-id={speakerId} data-speaker-tone={tone} style={style}>{name}{unconfirmed&&<small className="speaker-unset">名前未設定</small>}</span>;
}
