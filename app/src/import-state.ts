import { validVocabulary, type VocabularyEntry } from "./vocabulary-state.ts";
import type { StageProgress } from "./stage-progress.ts";

export interface AudioInfo { schemaVersion:number;detectedFormat:string;codec:string;sampleRate:number;channels:number;frames:number;durationUs:number }
export interface ImportSnapshot {
  revision:number;phase:string;busy:boolean;recordingBlocked:boolean;token:string|null;
  fileName:string|null;info:AudioInfo|null;sourceBytes:number|null;requiredBytes:number|null;availableBytes:number|null;
  completed:number;progress?:StageProgress|null;saved:{id:string;meetingId:string;state:string}|null;errorMessage:string|null;
}
export function acceptImport(current:ImportSnapshot|null,next:ImportSnapshot):ImportSnapshot {
  return current&&current.revision>next.revision?current:next;
}
export const importLabel=(phase:string)=>({idle:"音声を選択",choosing:"ファイル選択中",inspecting:"内容と長さを確認中",ready:"取り込み内容を確認",waiting:"解析の停止を待機中",copying:"原本をコピー中",decoding:"音声を復号中",converting:"解析用音声を準備中",saving:"保存を確定中",saved:"音声を保存しました",cancelling:"安全に取消中",cancelled:"取消済み",failed:"取り込みに要対応",recovering:"前回の取り込みを検証中"} as Record<string,string>)[phase]??phase;
export const bytesLabel=(value:number|null)=>value===null?"—":`${(value/1024/1024).toLocaleString("ja-JP",{maximumFractionDigits:1})} MiB`;
export function canStartImport(s:ImportSnapshot|null,title:string,terms:VocabularyEntry[],globalVersion:number|null,recordingActive:boolean):boolean {
  return !!s&&s.phase==="ready"&&!s.busy&&!s.recordingBlocked&&!recordingActive&&!!s.token&&!!s.info
    &&s.requiredBytes!==null&&s.availableBytes!==null&&s.availableBytes>=s.requiredBytes
    &&!!title.trim()&&[...title.trim()].length<=200&&validVocabulary(terms)&&globalVersion!==null&&Number.isSafeInteger(globalVersion)&&globalVersion>=0;
}
export function droppedAudio(paths:string[]):string {
  if(paths.length!==1||!paths[0])throw Error("音声ファイルを1件だけドロップしてください。");
  return paths[0]; // The backend verifies local path, actual content and duration.
}
