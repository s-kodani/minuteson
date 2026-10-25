import test from "node:test";
import assert from "node:assert/strict";
import { checkedResidencyState } from "../src/residency-state.ts";
const state={supported:true,canBackground:true,trayAvailable:true,dockAvailable:false,error:null};
test("background storage requires a verified native return path",()=>{
  assert.deepEqual(checkedResidencyState(state),state);
  assert.equal(checkedResidencyState({...state,trayAvailable:false,dockAvailable:true}).canBackground,true);
  assert.equal(checkedResidencyState({...state,canBackground:false,trayAvailable:false,error:"表示を確認できません"}).canBackground,false);
  for(const input of [null,{}, {...state,trayAvailable:false},{...state,supported:false},{...state,canBackground:null},{...state,error:7},{...state,error:"bad\0text"},{...state,force:true}])assert.throws(()=>checkedResidencyState(input));
});
