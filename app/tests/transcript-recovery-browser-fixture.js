// Synthetic journal CAS with real Web Crypto hashes. This is not native storage.
import { canonicalPayload } from "../src/transcript-state.ts";
export async function fixtureHash(value) {
  const digest = await crypto.subtle.digest("SHA-256",new TextEncoder().encode(JSON.stringify(value)));
  return Array.from(new Uint8Array(digest),byte => byte.toString(16).padStart(2,"0")).join("");
}
export async function stampFixtureDraft(draft) {
  if(draft) draft.contentHash = await fixtureHash(canonicalPayload(draft.payload));
  return structuredClone(draft);
}
export function recoveryFixture({ drafts, current, blocked = () => false }) {
  const f = { records: {},failRead: false,failSave: false,lostSave: false,lostClear: false,lostDiscard: false,failDiscard: false,
    delayMs: 0,calls: [],saveWait: false,releaseSave: null };
  const owner = async args => {
    const draft = drafts().find(d => d?.id === args.draftId && d.meetingId === args.meetingId && d.baseAnalysisId === args.baseAnalysisId);
    if(!draft) throw Error("模擬回復記録の所有不一致");
    await stampFixtureDraft(draft); return draft;
  };
  const read = async (draft,record) => record ? { ...structuredClone(record),
    currentDraftLockVersion: draft.lockVersion,currentDraftContentHash: draft.contentHash,
    matchesDraft: draft.lockVersion === record.baseDraftLockVersion && draft.contentHash === record.baseDraftContentHash,
    currentContext: current()?.id === draft.id && current()?.baseAnalysisId === draft.baseAnalysisId } : null;
  const write = async (draft,req,payload) => {
    const record = { meetingId: draft.meetingId,draftId: draft.id,baseAnalysisId: draft.baseAnalysisId,operationId: req.operationId,
      baseDraftLockVersion: draft.lockVersion,baseDraftContentHash: draft.contentHash,payload: structuredClone(payload),
      lockVersion: req.expectedRecoveryLockVersion + 1,contentHash: await fixtureHash(payload),updatedAt: Date.now() };
    f.records[draft.id] = record; return read(draft,record);
  };
  f.invoke = async (command,args) => {
    if(!["get_transcript_edit_recovery","save_transcript_edit_recovery","discard_transcript_edit_recovery","list_transcript_edit_recoveries"].includes(command)) return undefined;
    f.calls.push({ command,args: structuredClone(args) });
    if(command === "list_transcript_edit_recoveries") {
      if(f.failRead) throw Error("模擬回復読込失敗");
      const selected = drafts().filter(d => d?.meetingId === args.meetingId && f.records[d.id] && (!args.after || d.id > args.after)).sort((a,b) => a.id.localeCompare(b.id));
      const items = [];
      for(const draft of selected.slice(0,20)) { await stampFixtureDraft(draft); const r = await read(draft,f.records[draft.id]); items.push({ draftId: r.draftId,baseAnalysisId: r.baseAnalysisId,operationId: r.operationId,
        contentHash: r.contentHash,lockVersion: r.lockVersion,updatedAt: r.updatedAt,bufferCount: r.payload.buffers.length,matchesDraft: r.matchesDraft,currentContext: r.currentContext }); }
      return { items,nextCursor: selected.length > 20 ? items.at(-1).draftId : null };
    }
    const req = args.request ?? args, draft = await owner(req), old = f.records[draft.id];
    if(command === "get_transcript_edit_recovery") { if(f.failRead) throw Error("模擬回復読込失敗"); return read(draft,old); }
    if(blocked()) throw Error("模擬回復書込停止");
    if(command === "save_transcript_edit_recovery") {
      if(f.delayMs) await new Promise(resolve => setTimeout(resolve,f.delayMs));
      if(f.saveWait) { f.saveWait = false; await new Promise(resolve => { f.releaseSave = resolve; }); }
      if(f.failSave) throw Error("模擬回復保存失敗");
      if(current()?.id !== draft.id) throw Error("模擬現在下書き不一致");
      const buffers = structuredClone(req.buffers).map(b => ({ ...b,speakerIds: [...b.speakerIds].sort() })).sort((a,b) => a.utteranceId.localeCompare(b.utteranceId));
      if(old?.operationId === req.operationId) {
        if(old.payload.discardedFrom || old.lockVersion !== req.expectedRecoveryLockVersion + 1 || old.baseDraftLockVersion !== req.expectedDraftLockVersion || JSON.stringify(old.payload.buffers) !== JSON.stringify(buffers)) throw Error("模擬回復同一要求の変更");
        return read(draft,old);
      }
      if(draft.lockVersion !== req.expectedDraftLockVersion || (old?.lockVersion ?? 0) !== req.expectedRecoveryLockVersion) throw Error("模擬回復版競合");
      const saved = await write(draft,req,{ schemaVersion: 1,basePayload: canonicalPayload(draft.payload),buffers });
      if(f.lostSave || f.lostClear && !buffers.length) { f.lostSave = false; f.lostClear = false; throw Error("模擬回復保存応答喪失"); }
      return saved;
    }
    if(f.failDiscard) throw Error("模擬回復破棄失敗");
    const proof = { contentHash: req.expectedRecoveryContentHash,lockVersion: req.expectedRecoveryLockVersion };
    if(old?.operationId === req.operationId) {
      if(old.lockVersion !== req.expectedRecoveryLockVersion + 1 || JSON.stringify(old.payload.discardedFrom) !== JSON.stringify(proof)
        || old.baseDraftLockVersion !== req.expectedDraftLockVersion || old.baseDraftContentHash !== req.expectedDraftContentHash || old.payload.buffers.length) throw Error("模擬破棄同一要求の変更");
      return read(draft,old);
    }
    if(!old || old.lockVersion !== req.expectedRecoveryLockVersion || old.contentHash !== req.expectedRecoveryContentHash
      || draft.lockVersion !== req.expectedDraftLockVersion || draft.contentHash !== req.expectedDraftContentHash) throw Error("確認後に回復記録が変更されています。保持しています");
    const saved = await write(draft,req,{ schemaVersion: 2,basePayload: canonicalPayload(draft.payload),buffers: [],discardedFrom: proof });
    if(f.lostDiscard) { f.lostDiscard = false; throw Error("模擬回復破棄応答喪失"); }
    return saved;
  };
  return f;
}
