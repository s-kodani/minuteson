// Closed-recording delivery under synthetic IPC. No device, files or audio.
const clone=value=>structuredClone(value);
const valid=value=>/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value);
export const archivedFixture=()=>({rows:[],failRead:false,holdRead:false,reads:[],hold:{receive:false,render:false,close:false},lost:{receive:false,render:false,close:false},replies:[]});
export function newArchivedRow(data,{sessionId=crypto.randomUUID(),title="保存済み会議",count=1,both=false,legacy=false,issue=null,loss=0,historySaveIssue=null,deliveryUnavailable=false}={}){
  const meetingId=crypto.randomUUID();data.meetings.push({id:meetingId,title,lifecycle:"recorded",createdAt:Date.now(),updatedAt:Date.now(),lockVersion:0});
  const events=input=>Array.from({length:count*2},(_,i)=>{
    const sequence=i+1,interruptions=Math.ceil(sequence/2),kind=sequence%2?"interrupted":"recovered";
    return {id:`${sessionId}:${input}:${interruptions}:${kind}`,input,target:"main",kind,interruptions,recipientId:null,claimedAt:null,
      delivery:{sequence,generation:1,leaseAgeMs:null,rendered:false,receiveId:null,status:{state:sequence%2?"disconnected":"connected",sourceEpoch:Math.floor(sequence/2)+1,interruptions,reason:sequence%2?(input==="microphone"?"device_disconnected":"stream_stopped"):null}}};
  });
  return {meetingId,sessionId,title,revision:1,lanes:{microphone:events("microphone"),system:both?events("system"):[]},closed:{},loss:{microphone:loss,system:both?loss:0},historyIssue:issue,unavailable:false,legacy,historySaveIssue,deliveryUnavailable};
}
function project(head){const {claimedAt,...n}=head;return {...clone(n),delivery:{...clone(n.delivery),leaseAgeMs:claimedAt===null?null:Math.max(0,Date.now()-claimedAt)}};}
function expire(row){for(const lane of Object.values(row.lanes))while(lane.length){const h=lane[0];if(h.claimedAt===null||Date.now()-h.claimedAt<5000)break;
  row.revision++;if(h.delivery.rendered){lane.shift();continue;}h.delivery.generation++;h.recipientId=null;h.claimedAt=null;h.delivery.receiveId=null;break;
}}
export function reopenArchived(rows){for(const row of rows){row.closed={};for(const lane of Object.values(row.lanes)){
  while(lane[0]?.delivery.rendered)lane.shift();if(lane.length){lane[0].delivery.generation++;lane[0].recipientId=null;lane[0].claimedAt=null;lane[0].delivery.receiveId=null;}
}row.revision++;}return rows;}
export async function archivedPage(f,{afterSessionId}){
  const state=f.archived;if(afterSessionId!==null&&!valid(afterSessionId))throw Error("模擬続き位置不正");
  if(state.failRead)throw Error("模擬保存後通知読込み失敗");
  const rows=state.rows.filter(r=>afterSessionId===null||r.sessionId>afterSessionId).sort((a,b)=>a.sessionId.localeCompare(b.sessionId));
  const recordings=rows.slice(0,32).map(row=>{expire(row);return {meetingId:row.meetingId,sessionId:row.sessionId,title:row.title,
    batch:row.legacy||row.unavailable?null:{schemaVersion:1,ownerId:row.sessionId,revision:row.revision,notices:row.deliveryUnavailable?[]:Object.values(row.lanes).filter(l=>l.length).map(l=>project(l[0])),noticeLoss:clone(row.loss),historyIssue:row.historyIssue},issue:row.unavailable?"unavailable":row.deliveryUnavailable?"deliveryUnavailable":null,historySaveIssue:row.historySaveIssue??null};});
  const result={schemaVersion:3,recordings,nextCursor:rows.length>32?recordings.at(-1).sessionId:null};
  if(state.holdRead)await new Promise(resolve=>state.reads.push(resolve));return clone(result);
}
export async function archivedAction(f,args){
  const state=f.archived,action=args.action;
  if(state.hold[action])await new Promise(resolve=>state.replies.push({action,resolve}));
  const row=state.rows.find(r=>r.sessionId===args.sessionId&&r.meetingId===args.meetingId);
  if(!row||row.unavailable||row.deliveryUnavailable||state.failRead||!valid(args.recipientId)||new URLSearchParams(location.search).get("window"))throw Error("模擬保存後通知所有不正");
  expire(row);const prior=row.closed[args.noticeId];if(action!=="receive"&&prior?.recipientId===args.recipientId&&prior.delivery.generation===args.generation)return clone(prior);
  const lane=Object.values(row.lanes).find(l=>l[0]?.id===args.noticeId),head=lane?.[0];
  if(!head||head.delivery.generation!==args.generation)throw Error("模擬保存後通知の世代変更");
  if(action==="receive"){
    if(!valid(args.receiveId))throw Error("模擬受付操作ID不正");head.recipientId??=args.recipientId;
    if(head.recipientId===args.recipientId){head.claimedAt=Date.now();head.delivery.receiveId=args.receiveId;row.revision++;}
  }else{
    if(args.receiveId!==null||head.recipientId!==args.recipientId||head.claimedAt===null||action==="close"&&!head.delivery.rendered)throw Error("模擬表示確認なし");
    head.delivery.rendered=true;head.claimedAt=Date.now();row.revision++;
  }
  const n=project(head),result={schemaVersion:2,ownerId:row.sessionId,noticeId:n.id,target:"main",recipientId:n.recipientId,delivery:n.delivery};
  if(action==="close"){row.closed[head.id]=clone(result);lane.shift();}
  if(state.lost[action])throw Error("模擬確定後の応答喪失");return result;
}
