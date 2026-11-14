await import("./browser-fixture.js");
const { emit } = await import("@tauri-apps/api/event");
const fixture=window.appUpdateFixture={
  status:{revision:1,channel:"windows_store",state:"available",currentVersion:"1.0.1",updateAvailable:true,availableVersion:null,checkedAtMs:1794600000000,errorMessage:null},
  calls:[],holdRead:false,pendingReads:[],failOpen:false,
  async publish(patch){this.status={...this.status,...patch,revision:this.status.revision+1};await emit("app-update-status-changed",structuredClone(this.status));},
};
const base=window.__TAURI_INTERNALS__.invoke;
window.__TAURI_INTERNALS__.invoke=async(command,args,options)=>{
  if(command==="plugin:app|name")return "MinutesOn";
  if(command==="plugin:app|version")return "1.0.1";
  if(command==="get_app_update_status"){
    const snapshot=structuredClone(fixture.status);
    if(fixture.holdRead)await new Promise(resolve=>fixture.pendingReads.push(resolve));
    return snapshot;
  }
  if(command==="check_app_update"){
    fixture.calls.push(command);
    await fixture.publish({state:"checking",errorMessage:null});
    return structuredClone(fixture.status);
  }
  if(command==="open_app_update_page"){
    fixture.calls.push(command);
    if(fixture.failOpen)throw Error("synthetic launch error");
    return null;
  }
  return base(command,args,options);
};
