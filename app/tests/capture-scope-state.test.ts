import test from "node:test";
import assert from "node:assert/strict";
import {checkedCaptureApplications,checkedCaptureScope,captureApplicationScope,captureApplicationKey,type CaptureApplications,type CaptureScope} from "../src/capture-scope-state.ts";
const catalog=():CaptureApplications=>({schemaVersion:1,systemPermission:"unknown",applications:[{pid:123,bundleIdentifier:"com.example.browser",launchTimeUs:1_791_100_000_000_000,name:"同じアプリ名"},{pid:456,bundleIdentifier:"com.example.other",launchTimeUs:1_791_100_000_000_001,name:"同じアプリ名"}]});
test("capture scope binds exact process instance and never expands a missing target to global capture",()=>{
  const c=checkedCaptureApplications(catalog()),selected=c.applications[0];
  const scope:CaptureScope={mode:"application",application:structuredClone(selected)};
  assert.equal(captureApplicationScope(scope,c)?.pid,123);
  const expected={pid:selected.pid,bundleIdentifier:selected.bundleIdentifier,launchTimeUs:selected.launchTimeUs};
  assert.deepEqual(checkedCaptureScope(scope,c),{mode:"application",application:expected});
  assert.notEqual(captureApplicationKey(selected),captureApplicationKey(c.applications[1]));
  c.applications[0].launchTimeUs++;
  assert.equal(captureApplicationScope(scope,c),null);
  assert.throws(()=>checkedCaptureScope(scope,c),/全体取得への切替はしていません/);
  assert.throws(()=>checkedCaptureScope(scope,null));
});
test("global capture is independent of application availability and explicitly has no target",()=>{
  assert.deepEqual(checkedCaptureScope({mode:"system",application:null},null),{mode:"system",application:null});
  assert.throws(()=>checkedCaptureScope({mode:"system",application:catalog().applications[0]},catalog()));
  assert.throws(()=>checkedCaptureScope({mode:"all",application:null},catalog()));
  assert.throws(()=>checkedCaptureScope(null,catalog()));
});
test("application catalog cannot fabricate permission or accept ambiguous identities and oversized fields",()=>{
  for(const permission of ["authorized","permissionRequired","unknown"] as const){const c=catalog();c.systemPermission=permission;assert.equal(checkedCaptureApplications(c).systemPermission,permission);}
  for(const v of [null,{}, {...catalog(),schemaVersion:2},{...catalog(),systemPermission:"allowed"},{...catalog(),applications:null},{...catalog(),applications:Array.from({length:129},(_,pid)=>({...catalog().applications[0],pid:pid+1}))}])assert.throws(()=>checkedCaptureApplications(v));
  for(const patch of [{pid:0},{pid:-1},{pid:2_147_483_648},{pid:1.5},{pid:NaN},{launchTimeUs:0},{launchTimeUs:Number.MAX_SAFE_INTEGER+1},{launchTimeUs:Infinity},{bundleIdentifier:""},{bundleIdentifier:"a\nb"},{bundleIdentifier:"x".repeat(1025)},{name:""},{name:"日".repeat(342)}]){const c=catalog();Object.assign(c.applications[0],patch);assert.throws(()=>checkedCaptureApplications(c));}
  const duplicate=catalog();duplicate.applications[1].pid=duplicate.applications[0].pid;assert.throws(()=>checkedCaptureApplications(duplicate));
});
