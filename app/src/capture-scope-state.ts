// Application identity is a process instance, not just a reusable PID or name.
export type CaptureApplicationIdentity={pid:number;bundleIdentifier:string;launchTimeUs:number};
export type CaptureApplication=CaptureApplicationIdentity&{name:string};
export type CaptureApplications={schemaVersion:1;systemPermission:"authorized"|"permissionRequired"|"unknown";applications:CaptureApplication[]};
export type CaptureScope={mode:"system";application:null}|{mode:"application";application:CaptureApplicationIdentity};
const safeInteger=(value:unknown):value is number=>typeof value==="number"&&Number.isSafeInteger(value)&&value>0;
const text=(value:unknown):value is string=>typeof value==="string"&&!!value.trim()&&new TextEncoder().encode(value).length<=1024;
export function checkedCaptureApplicationIdentity(value:unknown):CaptureApplicationIdentity{
  const v=value as CaptureApplicationIdentity;
  if(!v||typeof v!=="object"||!safeInteger(v.pid)||v.pid>2_147_483_647||!safeInteger(v.launchTimeUs)||!text(v.bundleIdentifier)||/[\u0000-\u001f\u007f-\u009f]/u.test(v.bundleIdentifier))throw Error("録音対象アプリの識別情報が不正です。");
  return {pid:v.pid,bundleIdentifier:v.bundleIdentifier,launchTimeUs:v.launchTimeUs};
}
export const captureApplicationKey=(value:CaptureApplicationIdentity)=>JSON.stringify([value.pid,value.bundleIdentifier,value.launchTimeUs]);
export function checkedCaptureApplications(value:unknown):CaptureApplications{
  const v=value as CaptureApplications;
  if(!v||typeof v!=="object"||v.schemaVersion!==1||!["authorized","permissionRequired","unknown"].includes(v.systemPermission)||!Array.isArray(v.applications)||v.applications.length>128)throw Error("録音対象アプリ一覧の形式が不正です。");
  const pids=new Set<number>();
  const applications=v.applications.map(application=>{
    const identity=checkedCaptureApplicationIdentity(application);
    if(!text(application.name)||pids.has(identity.pid))throw Error("録音対象アプリ一覧の識別情報が不正です。");
    pids.add(identity.pid);
    return {...identity,name:application.name};
  });
  return {schemaVersion:1,systemPermission:v.systemPermission,applications};
}
export function captureApplicationScope(scope:CaptureScope,inputs:CaptureApplications|null):CaptureApplication|null{
  if(scope.mode!=="application"||!inputs)return null;
  const key=captureApplicationKey(checkedCaptureApplicationIdentity(scope.application));
  return inputs.applications.find(application=>captureApplicationKey(application)===key)??null;
}
export function checkedCaptureScope(value:unknown,inputs:CaptureApplications|null):CaptureScope{
  const v=value as CaptureScope;
  if(!v||typeof v!=="object")throw Error("PC音声の取得範囲が不正です。");
  if(v.mode==="system"&&v.application===null)return {mode:"system",application:null};
  if(v.mode!=="application")throw Error("PC音声の取得範囲が不正です。");
  const selected={mode:"application" as const,application:checkedCaptureApplicationIdentity(v.application)};
  if(!captureApplicationScope(selected,inputs))throw Error("確認したアプリが終了または再起動しました。一覧を再確認して明示選択してください。全体取得への切替はしていません。");
  return selected;
}
