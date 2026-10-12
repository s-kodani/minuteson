import { assertMarkdownDocument, type MarkdownDocument, type MarkdownStamp } from "./minutes-markdown-state.ts";

export interface MarkdownExportRequest { meetingId:string; operationId:string; expected:MarkdownStamp }
export interface MarkdownExportRecord {
  id:string; meetingId:string; minutesRevisionId:null; exportedContentHash:string;
  outputContentHash:string; outputSizeBytes:number; destinationPath:string;
  state:"writing"|"succeeded"|"failed"|"interrupted";
  errorMessage:string|null; createdAt:number; exportedAt:number|null;
}
export interface MarkdownExportStatus { active:boolean; record:MarkdownExportRecord|null }
type Pending = { request:MarkdownExportRequest; outputHash:string; size:number };

/** Explicit actions only. A response failure switches the same icon to a
 * read-only result check; no timer, focus effect or check reopens a save panel. */
export class MarkdownExportController {
  private pending:Pending|null=null;
  private inFlight:Promise<MarkdownExportRecord|null>|null=null;
  private needsCheck=false;
  private error="";
  private listeners=new Set<()=>void>();
  private readonly meetingId:string;
  private readonly send:(request:MarkdownExportRequest)=>Promise<MarkdownExportRecord|null>;
  private readonly read:(request:MarkdownExportRequest)=>Promise<MarkdownExportStatus>;
  private readonly operation:()=>string;
  constructor(meetingId:string,
    send:(request:MarkdownExportRequest)=>Promise<MarkdownExportRecord|null>,
    read:(request:MarkdownExportRequest)=>Promise<MarkdownExportStatus>,
    operation=()=>crypto.randomUUID()) {this.meetingId=meetingId;this.send=send;this.read=read;this.operation=operation;}
  snapshot(){return {busy:this.inFlight!==null,needsCheck:this.needsCheck,retry:this.pending!==null&&!this.needsCheck,error:this.error};}
  subscribe(listener:()=>void){this.listeners.add(listener);return()=>{this.listeners.delete(listener);};}
  private notify(){for(const listener of this.listeners)listener();}
  private validate(record:MarkdownExportRecord,pending:Pending) {
    if(!record||record.id!==pending.request.operationId||record.meetingId!==this.meetingId
      ||record.minutesRevisionId!==null||record.exportedContentHash!==pending.request.expected.contentHash
      ||record.outputContentHash!==pending.outputHash||record.outputSizeBytes!==pending.size
      ||typeof record.destinationPath!=="string"||!record.destinationPath||record.destinationPath.includes("\0")
      ||(record.errorMessage!==null&&typeof record.errorMessage!=="string")
      ||!Number.isSafeInteger(record.createdAt)||record.createdAt<0
      ||!["writing","succeeded","failed","interrupted"].includes(record.state)
      ||(record.state==="succeeded" ? !Number.isSafeInteger(record.exportedAt)||record.exportedAt!<0||record.errorMessage!==null : record.exportedAt!==null)) {
      throw Error("書き出し結果と保存済み本文が一致しません。再保存せず、結果を確認してください。");
    }
  }
  private finish(record:MarkdownExportRecord|null) {
    if(record){this.validate(record,this.pending!);if(record.state==="writing")throw Error("書き出し結果はまだ未確定です。結果を確認してください。");}
    this.pending=null;this.needsCheck=false;
    this.error=record&&record.state!=="succeeded" ? record.errorMessage||"書き出しの完了を確認できません。保存先を確認してください。" : "";
    return record;
  }
  private perform(action:()=>Promise<MarkdownExportRecord|null>) {
    if(this.inFlight)return this.inFlight;
    this.error="";
    this.inFlight=Promise.resolve().then(action).catch(cause=>{
      this.needsCheck=this.pending!==null;this.error=String(cause);throw cause;
    }).finally(()=>{this.inFlight=null;this.notify();});
    this.notify();return this.inFlight;
  }
  begin(document:MarkdownDocument):Promise<MarkdownExportRecord|null> {
    if(this.inFlight)return this.inFlight;
    if(this.needsCheck)return Promise.reject(Error("前回の書き出し結果を先に確認してください。"));
    return this.perform(async()=>{
      assertMarkdownDocument(this.meetingId,document);
      // Only after a read proves no active operation and no receipt may a
      // subsequent explicit click use a newer saved document. Otherwise an old
      // selection conflict would trap this icon in an unrecoverable retry loop.
      if(this.pending && (document.lockVersion!==this.pending.request.expected.lockVersion
        ||document.contentHash!==this.pending.request.expected.contentHash))this.pending=null;
      if(!this.pending){
        const request={meetingId:this.meetingId,operationId:this.operation(),expected:{lockVersion:document.lockVersion,contentHash:document.contentHash}};
        if(!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(request.operationId))throw Error("書き出し操作IDが不正です。");
        const bytes=new TextEncoder().encode(document.markdown);
        const hash=await crypto.subtle.digest("SHA-256",bytes);
        this.pending={request,size:bytes.length,outputHash:Array.from(new Uint8Array(hash),byte=>byte.toString(16).padStart(2,"0")).join("")};
      }
      return this.finish(await this.send(structuredClone(this.pending.request)));
    });
  }
  check():Promise<MarkdownExportRecord|null> {
    if(this.inFlight)return this.inFlight;
    if(!this.pending)return Promise.resolve(null);
    return this.perform(async()=>{
      const status=await this.read(structuredClone(this.pending!.request));
      if(!status||typeof status.active!=="boolean"||!("record" in status)
        ||(status.record!==null&&(!status.record||typeof status.record!=="object")))throw Error("書き出し状態を確認できません。");
      if(status.record){this.validate(status.record,this.pending!);if(status.record.state!=="writing")return this.finish(status.record);}
      if(status.active||status.record){this.needsCheck=true;this.error="書き出しは処理中または未確定です。結果を再確認してください。";return null;}
      // No live operation or receipt: allow only an explicit second click to
      // retry the identical source, or explicitly export a newer saved source.
      this.needsCheck=false;this.error="書き出し完了の記録がありません。保存先を確認し、必要なら書き出しを再試行してください。";return null;
    });
  }
}
