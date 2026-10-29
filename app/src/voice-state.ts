export interface VoiceSample { id:string;modelHash:string;extractorVersion:string;sourceMeetingId:string|null;sourceAvailable:boolean;startUs:number;endUs:number;createdAt:number }
export interface VoicePerson {id:string;name:string;version:number;samples:VoiceSample[]}
export interface VoiceMatch {id:string;speakerId:string;personId:string|null;nameSnapshot:string;state:"candidate"|"automatic"|"confirmed"|"dismissed";similarity:number|null;modelHash:string}
export interface VoiceWindow {speakerId:string;trackId:string;startUs:number;endUs:number}
export interface VoiceCandidate extends VoiceWindow {utteranceId:string;text:string}
export interface VoiceContext {supported:boolean;analysisId:string;workingHash:string;bookHash:string;people:VoicePerson[];matches:VoiceMatch[];windows:VoiceWindow[];registrationCandidates:VoiceCandidate[]}
export const voiceWindowKey=(w:VoiceWindow)=>JSON.stringify([w.speakerId,w.trackId,w.startUs,w.endUs]);
export const initialVoiceSelection=(candidates:VoiceCandidate[])=>[...candidates].sort((a,b)=>(b.endUs-b.startUs)-(a.endUs-a.startUs)||a.startUs-b.startUs||a.trackId.localeCompare(b.trackId)||a.utteranceId.localeCompare(b.utteranceId)).slice(0,2).map(voiceWindowKey);
export function selectedVoiceWindows(candidates:VoiceCandidate[],speakerId:string,keys:string[]):VoiceWindow[]{
  if(keys.length<1||keys.length>2||new Set(keys).size!==keys.length)throw Error("登録区間を1〜2区間選んでください。");
  return keys.map(key=>{
    const c=candidates.find(w=>w.speakerId===speakerId&&voiceWindowKey(w)===key);
    if(!c)throw Error("登録候補が変更されています。区間を選び直してください。");
    return {speakerId:c.speakerId,trackId:c.trackId,startUs:c.startUs,endUs:c.endUs};
  });
}
export const voiceMatchLabel=(state:VoiceMatch["state"])=>({candidate:"候補",automatic:"自動識別",confirmed:"確認済み",dismissed:"対応を解除済み"})[state];
export function registrationName(name:string):string {const clean=name.trim();if(!clean||Array.from(clean).length>200||/[\u0000-\u001f\u007f]/.test(clean))throw Error("人物名を1〜200文字で入力してください。");return clean;}
export function voiceStartRequest(meetingId:string,context:VoiceContext,speakerId:string|null,personId:string|null,name:string,operationId:string,selectedKeys?:string[]){
  if(!context.supported)throw Error("この環境では声の登録・照合を利用できません。");
  if(speakerId&&!(selectedKeys===undefined?context.windows:context.registrationCandidates).some(w=>w.speakerId===speakerId))throw Error("登録に使える単独の発言がありません。");
  const selectedWindows=speakerId&&selectedKeys!==undefined?selectedVoiceWindows(context.registrationCandidates,speakerId,selectedKeys):undefined;
  if(personId&&!context.people.some(p=>p.id===personId))throw Error("追加登録する人物が見つかりません。");
  return {meetingId,operationId,analysisId:context.analysisId,workingHash:context.workingHash,bookHash:context.bookHash,speakerId,
    personId:speakerId?personId:null,personName:speakerId&&!personId?registrationName(name):null,...(selectedWindows?{selectedWindows}:{})};
}

export interface VoiceSamplePlaybackRequest {voiceSampleId:string;expectedStartUs:number;expectedEndUs:number}
