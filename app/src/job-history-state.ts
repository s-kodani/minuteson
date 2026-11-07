export const historyKinds = ["recording_save","recording_recovery","model","audio_import","transcription","diarization","minutes_generation","voice_registration","voice_matching","speaker_completion","backup","restore","deletion","audio_deletion","model_deletion"] as const;
export const historyStates = { succeeded:"完了", failed:"失敗", cancelled:"取消済み", interrupted:"中断", attention:"要確認" } as const;
export type HistoryState = keyof typeof historyStates;
export interface JobHistoryEntry { id:string; kind:typeof historyKinds[number];target:string;meetingId:string|null;state:HistoryState;acceptedAt:number|null;finishedAt:number|null;observedAt:number;detail:string|null;relatedMeetingIds?:string[] }
export interface HistoryCursor { at:number;id:string }
export interface JobHistoryPage { items:JobHistoryEntry[];nextCursor:HistoryCursor|null;storageError:string|null }
export function checkedHistoryPage(value:unknown):JobHistoryPage {
  const page=value as JobHistoryPage;
  const time=(v:unknown)=>v===null||Number.isSafeInteger(v)&&Number(v)>=0;
  if(!page||!Array.isArray(page.items)||page.items.length>100||page.storageError!==null&&typeof page.storageError!=="string"
    ||page.nextCursor!==null&&(!page.nextCursor||!time(page.nextCursor.at)||page.nextCursor.at===null||typeof page.nextCursor.id!=="string")
    ||page.items.some(e=>!e||typeof e.id!=="string"||!e.id||!historyKinds.includes(e.kind)||!Object.hasOwn(historyStates,e.state)||typeof e.target!=="string"||e.meetingId!==null&&typeof e.meetingId!=="string"||!time(e.acceptedAt)||!time(e.finishedAt)||!time(e.observedAt)||e.observedAt===null||e.detail!==null&&typeof e.detail!=="string"||e.relatedMeetingIds!==undefined&&(!Array.isArray(e.relatedMeetingIds)||e.relatedMeetingIds.some(id=>typeof id!=="string")))
    ||new Set(page.items.map(e=>e.id)).size!==page.items.length)throw Error("ジョブ履歴の取得結果を確認できません。");
  return page;
}
export function mergeHistory(existing:JobHistoryEntry[],next:JobHistoryEntry[]) {const rows=new Map(existing.map(e=>[e.id,e]));for(const e of next)rows.set(e.id,e);return [...rows.values()];}
export function historyElapsed(entry:Pick<JobHistoryEntry,"acceptedAt"|"finishedAt">):string {
  if(entry.acceptedAt===null||entry.finishedAt===null||entry.finishedAt<entry.acceptedAt)return "—";
  const seconds=Math.floor((entry.finishedAt-entry.acceptedAt)/1000),h=Math.floor(seconds/3600),m=Math.floor(seconds%3600/60),s=seconds%60;
  return `${h?`${h}時間`:""}${m?`${m}分`:""}${s}秒`;
}
