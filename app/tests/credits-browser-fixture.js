// Production settings and modal with synthetic attribution resources.
await import("./browser-fixture.js");
const baseInvoke=window.__TAURI_INTERNALS__.invoke;
const fixture=window.creditsFixture={calls:0,fail:true,about:{calls:[],fail:true,hold:false,pending:[],name:"MinutesOn QA",version:"9.8.7-preview.2"}};
const entry=(name,version,license)=>({name,version,license,sources:[`https://example.test/${name}`]});
window.__TAURI_INTERNALS__.invoke=async(command,args,options)=>{
  if(["plugin:app|name","plugin:app|version"].includes(command)){
    const about=fixture.about;
    about.calls.push(command);
    if(about.hold)await new Promise(resolve=>about.pending.push(resolve));
    if(about.fail)throw Error("合成アプリ情報読込エラー");
    return command==="plugin:app|name"?about.name:about.version;
  }
  if(command!=="get_credits")return baseInvoke(command,args,options);
  fixture.calls++;
  if(fixture.fail)throw Error("合成クレジット読込エラー");
  return {
    technologies:[entry("React","19.0.0","MIT"),entry("llama.cpp","b11533","MIT")],
    models:[entry("Qwen3-4B","fixed-model-version","Apache-2.0")],
    dependencies:Array.from({length:80},(_,i)=>entry(`dependency-${i}`,"1.2.3","MIT")),
    notices:[{title:"依存ライブラリの通知文",text:`Synthetic notice\n${"All original attribution text remains readable.\n".repeat(300)}<script>window.creditMarkupExecuted=true</script>`}],
  };
};
