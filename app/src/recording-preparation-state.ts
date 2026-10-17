// A preparation is local UI state, not a saved meeting or a capture session.
export interface RecordingPreparation { id:string; title:string; submitted:boolean; speakerCounts:{mic:number|null;system:number|null} }

export function recordingTitle(now:Date):string {
  if(!Number.isFinite(now.getTime()))throw Error("会議日時が不正です。");
  const pad=(value:number)=>String(value).padStart(2,"0");
  return `${now.getFullYear()}/${pad(now.getMonth()+1)}/${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}の会議`;
}

export function validRecordingTitle(title:string):boolean {
  const length=Array.from(title.trim()).length;
  return length>0&&length<=200;
}

export function newRecordingPreparation(id:string,now:Date):RecordingPreparation {
  if(!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(id))throw Error("録音準備の識別子が不正です。");
  return {id,title:recordingTitle(now),submitted:false,speakerCounts:{mic:null,system:null}};
}

export function sameRecordingPreparation(current:RecordingPreparation|null,expected:RecordingPreparation):boolean {
  return !!current&&!current.submitted&&current.id===expected.id&&current.title.trim()===expected.title.trim()
    &&current.speakerCounts.mic===expected.speakerCounts.mic&&current.speakerCounts.system===expected.speakerCounts.system;
}
