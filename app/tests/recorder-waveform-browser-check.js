// Production mini panel, synthetic telemetry only; no native audio capture.
async (page,capturePath=path=>`output/recorder-panel-ui/${path}`)=>{
  const results=[],check=(ok,message)=>{if(!ok)throw Error(message);};
  // Install before the fixture creates timers; let startup load naturally,
  // then advance telemetry and freshness through every callback with runFor.
  await page.clock.install({time:new Date("2026-11-01T00:00:00.000Z")});
  await page.context().route(/^http:\/\/127\.0\.0\.1:1420\/(?:\?.*)?$/,async route=>{
    const response=await route.fetch(),body=(await response.text()).replace(/<script type="module" src="\/src\/bootstrap\.tsx[^"]*"><\/script>/u,'<script type="module">await import("/tests/application-close-browser-fixture.js");</script>');
    await route.fulfill({response,body});
  });
  await page.setViewportSize({width:227,height:101});
  await page.goto("http://127.0.0.1:1420/?window=recorder");
  await page.waitForFunction(()=>!!window.closeFixture);
  const report=async(microphone,system)=>{await page.evaluate(([microphone,system])=>{
    const f=window.closeFixture,track=peak=>typeof peak==="string"?{state:peak,windowFrames:null,ageMs:null,rms:null,peak:null}:{state:"measured",windowFrames:4800,ageMs:0,rms:peak/2,peak};
    f.inputLevels={schemaVersion:1,ownerId:f.recording.sessionId,revision:(f.inputLevels?.revision??0)+1,reportAgeMs:0,microphone:track(microphone),system:track(system)};
  },[microphone,system]);await page.clock.runFor(100);};
  const states=(m,s)=>page.waitForFunction(([m,s])=>{const waves=document.querySelectorAll('.mini-waveform');return waves[0]?.dataset.levelState===m&&waves[1]?.dataset.levelState===s;},[m,s]);
  const bars=input=>page.locator(`[data-source="${input}"] .mini-waveform line`);
  await page.evaluate(async()=>{
    const f=window.closeFixture;
    await f.recordChanged({phase:"recording",sessionId:crypto.randomUUID(),meetingId:crypto.randomUUID(),elapsedMs:24000});
    f.refreshInputStatus=true;
    f.queueState={lanes:{microphone:[],system:[]},closed:{},loss:{microphone:0,system:0}};
    const binding={state:"connected",sourceEpoch:1,interruptions:0,reason:null};
    f.inputStatus={schemaVersion:4,ownerId:f.recording.sessionId,revision:1,reportAgeMs:0,microphone:binding,system:binding,notices:[],noticeLoss:{microphone:0,system:0}};
  });
  await page.waitForFunction(()=>document.querySelector('.mini-input-indicator')?.dataset.inputState==="connected");
  await page.clock.pauseAt(new Date("2026-11-01T01:00:00.000Z"));
  await report("waiting","waiting");await states("waiting","waiting");
  check(await bars("microphone").count()===0&&await bars("system").count()===0,"connected inputs invented a waveform");
  results.push("connection without received samples shows no invented waveform");
  await report(.25,.81);await states("measured","measured");
  await page.waitForFunction(()=>document.querySelectorAll('.mini-waveform line').length===2);
  check(Number(await bars("microphone").getAttribute("y1"))===10&&Math.abs(Number(await bars("system").getAttribute("y1"))-3.6)<.01,"separate peaks did not drive separate bars");
  await page.clock.runFor(250);
  check(await bars("microphone").count()===1,"polling the same revision duplicated waveform data");
  results.push("separate native measurements drive gray bars without duplicating immutable revisions");
  await report(0,"waiting");await states("measured","waiting");
  await page.waitForFunction(()=>document.querySelectorAll('[data-source="microphone"] .mini-waveform line').length===2);
  check(await bars("microphone").last().getAttribute("y1")==="17"&&await bars("system").count()===0,"silence or missing PC audio fabricated a signal");
  check(!(await page.locator('.mini-shell').innerText()).match(/要対応|マイク|PC音声|録音中|未受信|確認不可/),"absent PC signal added unwanted text");
  check(await page.getByRole('img',{name:'マイク',exact:true}).count()===1&&await page.getByRole('img',{name:'PC音声',exact:true}).count()===1,"source icons lost accessible names");
  results.push("received silence stays flat and a missing PC signal stays quiet without hiding its accessible icon");
  await page.clock.runFor(1100);await states("stale","waiting");
  check(await bars("microphone").count()===0,"stale telemetry retained a live waveform");
  results.push("expired telemetry removes the previous live waveform");
  await page.evaluate(()=>{
    const f=window.closeFixture,values=[.04,.09,.36,.81,.25,.16,.64,.49,.09,.01,.25,.64,.36,.16,.04,.49];let i=0;
    window.miniWaveformFixtureTimer=setInterval(()=>{
      const track=peak=>({state:"measured",windowFrames:4800,ageMs:0,rms:peak/2,peak});
      f.inputLevels={schemaVersion:1,ownerId:f.recording.sessionId,revision:(f.inputLevels?.revision??0)+1,reportAgeMs:0,microphone:track(values[i%values.length]),system:track(values[(i*3+5)%values.length])};i++;
    },100);
  });
  await page.clock.runFor(6500);
  await page.waitForFunction(()=>document.querySelectorAll('[data-source="microphone"] .mini-waveform line').length===64);
  for(const theme of ["light","dark"]){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    const geometry=await page.locator('.mini-shell').evaluate(shell=>{
      const grip=shell.querySelector('.mini-drag'),r=grip.getBoundingClientRect(),dot=shell.querySelector('.mini-drag-dots').getBoundingClientRect();
      const buttons=[...shell.querySelectorAll('.mini-bottom button')].map(b=>b.getBoundingClientRect());
      const wave=[...shell.querySelectorAll('.mini-waveform')].map(w=>getComputedStyle(w).color);
      return {center:(r.top+r.bottom)/2,dotCenter:(dot.top+dot.bottom)/2,border:getComputedStyle(grip).borderRightWidth,overflow:shell.scrollWidth>shell.clientWidth||shell.scrollHeight>shell.clientHeight,buttonsFit:buttons.every(b=>b.x>=0&&b.right<=innerWidth&&b.y>=0&&b.bottom<=innerHeight),colors:wave};
    });
    check(Math.abs(geometry.center-50.5)<.1&&Math.abs(geometry.dotCenter-50.5)<.1&&geometry.border==="0px"&&!geometry.overflow&&geometry.buttonsFit,`grip alignment, divider removal or controls invalid: ${JSON.stringify(geometry)}`);
    check(geometry.colors[0]===geometry.colors[1]&&["rgb(146, 149, 156)","rgb(161, 161, 170)"].includes(geometry.colors[0]),"waveforms are not matching gray tones");
    await page.screenshot({path:capturePath(`mini-panel-${theme}.png`)});
    results.push(`${theme}: centered six-dot grip has no divider; controls and matching gray waveforms fit 227×101`);
  }
  await page.evaluate(()=>{clearInterval(window.miniWaveformFixtureTimer);window.closeFixture.failInputLevels=true;});await page.clock.runFor(100);await states("unavailable","unavailable");
  check(await bars("microphone").count()===0&&await bars("system").count()===0,"failed level RPC retained old waveform");
  results.push("failed telemetry clears both waveforms");
  await page.evaluate(()=>window.closeFixture.recordChanged({phase:"stopping"}));
  await page.locator('.mini-stop').waitFor({state:"detached"});
  await page.clock.runFor(16);await states("unavailable","unavailable");
  const reads=await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length);
  await page.clock.runFor(250);
  check(await page.evaluate(()=>window.closeFixture.calls.filter(c=>c.command==="get_recording_levels").length)===reads,"stopping kept polling audio");
  results.push("stopping clears signals and stops telemetry polling");
  return {passed:results.length,results,nativeRecording:false};
}
