import test from "node:test";
import assert from "node:assert/strict";
import { checkedStartupNotices } from "../src/startup-notice-state.ts";
const information = {id:"information",revision:"a".repeat(64),kind:"information",title:"案内",message:"本文",priority:10,deferred:false,optionId:null,packageId:null,recipeHash:null};
const runtime = {id:"llama-runtime-update",revision:"b".repeat(64),kind:"model_runtime_update",title:"更新",message:"本文",priority:0,deferred:false,optionId:"llama-b11533-qwen3-4b-q4km",packageId:"11111111-1111-4111-8111-111111111111",recipeHash:"c".repeat(64)};
test("startup candidates preserve exact targets, native deferral and priority",()=>{
  assert.deepEqual(checkedStartupNotices([information,{...runtime,deferred:true}]),[{...runtime,deferred:true},information]);
  assert.deepEqual(checkedStartupNotices([]),[]);
});
test("invalid or duplicate startup notices cannot become actions",()=>{
  for(const value of [[runtime,runtime],[{...runtime,recipeHash:null}],[{...information,optionId:"execute"}],[{...runtime,kind:"execute"}],[{...runtime,revision:"old"}]])assert.throws(()=>checkedStartupNotices(value));
});
