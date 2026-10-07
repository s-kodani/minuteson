import test from "node:test";
import assert from "node:assert/strict";
import { assertUiPreferences, readingPreferences, ThemeController, type UiPreferencesSnapshot, type UiPreferencesPayload } from "../src/theme-state.ts";

const legacy = (): UiPreferencesSnapshot => ({id:"ui_preferences",payload:{schemaVersion:1,theme:"system"},lockVersion:0});
const full = (payload: Partial<Extract<UiPreferencesPayload,{schemaVersion:2}>> = {}, lockVersion = 1): UiPreferencesSnapshot => ({id:"ui_preferences",payload:{schemaVersion:2,theme:"system",textSize:15,followPlayback:false,...payload},lockVersion});
function deferred<T>() { let resolve!: (value:T)=>void; const promise=new Promise<T>(done=>{resolve=done;});return {promise,resolve}; }

test("display preferences read legacy as 15px and non-following without inventing stored fields",()=>{
  const old=legacy();assert.deepEqual(assertUiPreferences(old),old);
  assert.deepEqual(readingPreferences(old.payload),{textSize:15,followPlayback:false});
  for(const textSize of [13,15,18,20] as const) for(const followPlayback of [true,false]) {
    const value=full({theme:"dark",textSize,followPlayback});assert.deepEqual(assertUiPreferences(value),value);
    assert.deepEqual(readingPreferences(value.payload),{textSize,followPlayback});
  }
  for(const payload of [
    {schemaVersion:2,theme:"system",textSize:15},
    {schemaVersion:2,theme:"system",textSize:14,followPlayback:false},
    {schemaVersion:2,theme:"system",textSize:15,followPlayback:0},
    {schemaVersion:2,theme:"system",textSize:15,followPlayback:true,extra:null},
    {schemaVersion:1,theme:"system",textSize:null,followPlayback:null},
  ]) assert.throws(()=>assertUiPreferences({...full(),payload}));
  assert.throws(()=>assertUiPreferences(full({},0)));
});

test("display save upgrades only on explicit change and every later theme save preserves reading fields",async()=>{
  let stored=legacy();const writes:Array<{version:number,payload:UiPreferencesPayload}>=[];
  const c=new ThemeController({read:async()=>stored,save:async(version,theme)=>{throw Error(`legacy write not allowed ${version} ${theme}`);},savePayload:async(version,payload)=>{writes.push({version,payload});return stored={id:"ui_preferences",payload,lockVersion:version+1};}});
  await c.refresh();await c.refresh();assert.equal(writes.length,0);
  await c.changeDisplay({textSize:18});assert.deepEqual(stored,full({textSize:18},1));
  await c.changeDisplay({followPlayback:true});assert.deepEqual(stored,full({textSize:18,followPlayback:true},2));
  await c.change("dark");assert.deepEqual(stored,full({theme:"dark",textSize:18,followPlayback:true},3));
  assert.deepEqual(writes.map(w=>w.version),[0,1,2]);assert.equal(c.getSnapshot().pendingPreferences,null);
});

test("display failure retains only requested fields; explicit retry keeps newer theme and other reading field",async()=>{
  let stored=full(),fail=true,writes=0;const saved:Array<UiPreferencesPayload>=[];
  const c=new ThemeController({read:async()=>stored,save:async()=>{throw Error("legacy");},savePayload:async(version,payload)=>{writes++;saved.push(payload);if(fail)throw Error("disk full");return stored={id:"ui_preferences",payload,lockVersion:version+1};}});
  await c.refresh();await c.changeDisplay({textSize:20});
  assert.deepEqual(c.getSnapshot().pendingDisplay,{textSize:20});assert.equal(readingPreferences(c.getSnapshot().snapshot?.payload).textSize,15);
  await c.change("dark");await c.changeDisplay({followPlayback:true});await c.refresh();assert.equal(writes,1);
  stored=full({theme:"light",followPlayback:true},2);await c.refresh();fail=false;await c.retry();
  assert.deepEqual(saved.at(-1),{schemaVersion:2,theme:"light",textSize:20,followPlayback:true});
  assert.deepEqual(stored,full({theme:"light",textSize:20,followPlayback:true},3));
  assert.equal(writes,2);assert.equal(c.getSnapshot().pendingDisplay,null);
});

test("lost display response applies only authoritative saved settings and never blindly writes again",async()=>{
  let stored=full(),offline=false,writes=0;
  const c=new ThemeController({read:async()=>{if(offline)throw Error("offline");return stored;},save:async()=>{throw Error("legacy");},savePayload:async(version,payload)=>{writes++;stored={id:"ui_preferences",payload,lockVersion:version+1};offline=true;throw Error("lost committed response");}});
  await c.refresh();await c.changeDisplay({followPlayback:true});
  assert.equal(c.getSnapshot().uncertain,true);assert.equal(readingPreferences(c.getSnapshot().snapshot?.payload).followPlayback,false);
  await c.retry();await c.changeDisplay({textSize:18});await c.change("light");assert.equal(writes,1);
  offline=false;stored=full({theme:"dark",followPlayback:true},3);await c.refresh();
  assert.deepEqual(c.getSnapshot().snapshot,stored);assert.equal(c.getSnapshot().pendingPreferences,null);assert.equal(c.getSnapshot().notice?.level,"info");
  const notice=c.getSnapshot().notice;await c.refresh();assert.equal(c.getSnapshot().notice,notice);assert.equal(writes,1);
});

test("display settings reject same-version field changes, stale snapshots and mismatched full receipts",async()=>{
  let stored=full();const c=new ThemeController({read:async()=>stored,save:async()=>{throw Error("legacy");},savePayload:async()=>full({theme:"dark",textSize:13},2)});
  await c.refresh();await c.changeDisplay({textSize:20});assert.deepEqual(c.getSnapshot().snapshot,full());assert.deepEqual(c.getSnapshot().pendingDisplay,{textSize:20});
  stored=full({followPlayback:true},1);await c.refresh();assert.equal(c.getSnapshot().uncertain,true);assert.deepEqual(c.getSnapshot().snapshot,full());
  stored=legacy();await c.refresh();assert.equal(c.getSnapshot().snapshot?.lockVersion,1);
});

test("display saves block overlapping theme/display requests and discard stale in-flight reads",async()=>{
  const read=deferred<unknown>(),save=deferred<unknown>();let reads=0,writes=0;
  const c=new ThemeController({read:()=>++reads===1?Promise.resolve(full()):read.promise,save:async()=>{throw Error("legacy");},savePayload:()=>{writes++;return save.promise;}});
  await c.refresh();const stale=c.refresh(),writing=c.changeDisplay({textSize:18});
  await c.change("light");await c.changeDisplay({followPlayback:true});await c.refresh();assert.equal(writes,1);
  save.resolve(full({textSize:18},2));await writing;read.resolve(full());await stale;
  assert.deepEqual(c.getSnapshot().snapshot,full({textSize:18},2));assert.equal(c.getSnapshot().error,"");
});

test("explicit reload discards a failed display choice only after user action; malformed changes never write",async()=>{
  let writes=0;const c=new ThemeController({read:async()=>full(),save:async()=>{throw Error("legacy");},savePayload:async()=>{writes++;throw Error("disk full");}});
  await c.refresh();await c.changeDisplay({textSize:18});await c.refresh();assert.deepEqual(c.getSnapshot().pendingDisplay,{textSize:18});
  await c.reload();assert.equal(c.getSnapshot().pendingDisplay,null);assert.equal(c.getSnapshot().pendingPreferences,null);assert.equal(writes,1);
  for(const patch of [{},{textSize:14},{followPlayback:null},{theme:"dark"}]) await assert.rejects(()=>c.changeDisplay(patch as never));
  assert.equal(writes,1);
});

test("theme retry after a display upgrade uses current format and preserves all newer reading preferences",async()=>{
  let stored=legacy();const desired:UiPreferencesPayload[]=[];
  const c=new ThemeController({read:async()=>stored,save:async()=>{stored=full({theme:"light",textSize:20,followPlayback:true});throw Error("concurrent display upgrade");},savePayload:async(version,payload)=>{desired.push(payload);return stored={id:"ui_preferences",payload,lockVersion:version+1};}});
  await c.refresh();await c.change("dark");assert.equal(c.getSnapshot().pendingTheme,"dark");await c.retry();
  assert.deepEqual(desired,[{schemaVersion:2,theme:"dark",textSize:20,followPlayback:true}]);assert.equal(c.getSnapshot().pendingTheme,null);
});
