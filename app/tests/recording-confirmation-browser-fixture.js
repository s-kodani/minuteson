// Synthetic IPC forwarded to the single test controller in Playwright.
// No microphone, native process, user filesystem or persistent browser data.
import {mockIPC,mockWindows} from "./startup-ready-mocks.ts";
window.skipDataFixtureMain=true;
const {dataSaveIPC}=await import("./data-save-browser-fixture.js");
const label=new URLSearchParams(location.search).get("window")??"main";
mockWindows(label);
const rpc=async(command,args={})=>{
 const response=await fetch("/__synthetic_recording_controller",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({command,args,label})});
 const result=await response.json();if(result.error)throw Error(result.error);return result.value;
};
window.recordingFixture={read:()=>rpc("fixture-read"),reset:()=>rpc("fixture-reset"),configure:patch=>rpc("fixture-configure",patch),phase:phase=>rpc("fixture-phase",{phase}),finish:failed=>rpc("fixture-finish",{failed})};
mockIPC(async(command,args)=>{
 if(["get_recording_snapshot","get_recording_confirmation","request_recording_confirmation","decide_recording_confirmation"].includes(command))return rpc(command,args);
 return dataSaveIPC(command,args);
},{shouldMockEvents:true});
await import("../src/main.tsx");
