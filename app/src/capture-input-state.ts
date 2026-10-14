export type MicrophonePermission="authorized"|"denied"|"restricted"|"notDetermined"|"unknown";
export type CaptureInputDevice={uid:string;name:string;isDefault:boolean};
export type CaptureInputs={schemaVersion:1;microphonePermission:MicrophonePermission;devices:CaptureInputDevice[];selectedMicrophoneUID:string|null};
const bytes=(s:string)=>new TextEncoder().encode(s).length;
export const validMicrophoneUID=(s:unknown):s is string=>typeof s==="string"&&!!s.trim()&&bytes(s)<=1024&&!/[\u0000-\u001f\u007f-\u009f]/u.test(s);
export function checkedCaptureInputs(value:unknown):CaptureInputs {
  const v=value as CaptureInputs;
  if(!v||typeof v!=="object"||v.schemaVersion!==1||!["authorized","denied","restricted","notDetermined","unknown"].includes(v.microphonePermission)||!Array.isArray(v.devices)||v.devices.length>128||v.selectedMicrophoneUID!==null&&!validMicrophoneUID(v.selectedMicrophoneUID))throw Error("マイク一覧の形式が不正です。");
  const ids=new Set<string>();let defaults=0;
  for(const d of v.devices){
    if(!d||typeof d!=="object"||!validMicrophoneUID(d.uid)||typeof d.name!=="string"||!d.name.trim()||bytes(d.name)>1024||typeof d.isDefault!=="boolean"||ids.has(d.uid))throw Error("マイク一覧の識別情報が不正です。");
    ids.add(d.uid);if(d.isDefault)defaults++;
  }
  if(defaults>1)throw Error("既定のマイクを一意に確認できません。");
  return v;
}
// Only the very first choice may use the OS default. A remembered or explicit
// device that disappeared stays selected and requires the user's decision.
export function microphoneChoice(inputs:CaptureInputs,previous:string|null):string {
  return previous??inputs.selectedMicrophoneUID??inputs.devices.find(d=>d.isDefault)?.uid??"";
}
export function usableMicrophone(inputs:CaptureInputs|null,uid:string):CaptureInputDevice|null {
  if(!inputs||["denied","restricted"].includes(inputs.microphonePermission))return null;
  return inputs.devices.find(d=>d.uid===uid)??null;
}
export function usableAutomaticMicrophone(inputs:CaptureInputs|null):CaptureInputDevice|null {
  if(!inputs||["denied","restricted"].includes(inputs.microphonePermission))return null;
  return inputs.devices.find(d=>d.isDefault)??null;
}
