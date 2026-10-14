import {validMicrophoneUID} from "./capture-input-state.ts";
import {validInputOwner} from "./recording-input-state.ts";

export type MicrophoneSelectionStatus={schemaVersion:1;ownerId:string;revision:number;requestRevision:number;mode:"automatic"|"fixed";deviceUID:string;source:"application"|"systemDefault"|"fixed"|"pending";fallbackReason:"mon_direct_recording"|"application_input_unavailable"|"application_input_ambiguous"|null};
const safe=(value:unknown,min:number)=>typeof value==="number"&&Number.isSafeInteger(value)&&value>=min;
export function checkedMicrophoneSelectionStatus(value:unknown,owner:string):MicrophoneSelectionStatus{
  if(!value||typeof value!=="object"||Array.isArray(value)||!validInputOwner(owner))throw Error("マイク選択状態の対象が不正です。");
  const v=value as Record<string,unknown>;
  if(Object.keys(v).sort().join("|")!==["schemaVersion","ownerId","revision","requestRevision","mode","deviceUID","source","fallbackReason"].sort().join("|")||
    v.schemaVersion!==1||v.ownerId!==owner||!safe(v.revision,1)||!safe(v.requestRevision,0)||!validMicrophoneUID(v.deviceUID))throw Error("マイク選択状態の内容が不正です。");
  const fixed=v.mode==="fixed"&&(v.source==="fixed"||v.source==="pending")&&v.fallbackReason===null;
  const automatic=v.mode==="automatic"&&(v.source==="application"||v.source==="pending")&&v.fallbackReason===null;
  const defaultInput=v.mode==="automatic"&&v.source==="systemDefault"&&["mon_direct_recording","application_input_unavailable","application_input_ambiguous"].includes(v.fallbackReason as string);
  if(!fixed&&!automatic&&!defaultInput)throw Error("マイク選択状態の取得元が不正です。");
  return v as MicrophoneSelectionStatus;
}
export function microphoneFallbackMessage(status:MicrophoneSelectionStatus):string|null{
  if(status.source!=="systemDefault")return null;
  if(status.fallbackReason==="application_input_unavailable")return "会議アプリの使用マイクを確認できないため、OSの既定入力を選択しました。取得状態を確認してください。";
  if(status.fallbackReason==="application_input_ambiguous")return "会議アプリの使用マイクを一意に特定できないため、OSの既定入力を選択しました。取得状態を確認してください。";
  return null;
}
