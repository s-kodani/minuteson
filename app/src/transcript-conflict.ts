import { canonicalPayload, type DraftPayload } from "./transcript-state.ts";

export type ConflictChoice = "local" | "latest";
type Kind = "name" | "text" | "assignment" | "presence" | "manual" | "merge" | "split";
export interface TranscriptConflictRow {
  key: string; kind: Kind; id: string;
  base: unknown; local: unknown; latest: unknown;
  conflict: boolean; suggested: ConflictChoice | null;
}
export interface TranscriptConflictPlan {
  base: DraftPayload; local: DraftPayload; latest: DraftPayload;
  rows: TranscriptConflictRow[];
}
const equal = (a: unknown,b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const key = (kind: Kind,id: string) => JSON.stringify([kind,id]);
function immutable<T>(value: T): T {
  const copy = structuredClone(value);
  const freeze = (item: unknown) => {
    if(item && typeof item === "object") { for(const child of Object.values(item)) freeze(child); Object.freeze(item); }
  };
  freeze(copy); return copy;
}
function atoms(payload: DraftPayload) {
  const result = new Map<string,{ kind: Kind; id: string; value: unknown }>();
  const put = (kind: Kind,id: string,value: unknown) => {
    if(result.has(key(kind,id))) throw Error("比較する下書きに重複したIDがあります");
    result.set(key(kind,id),{ kind,id,value });
  };
  for(const row of payload.speaker_names) put("name",row.speaker_id,row.display_name);
  for(const row of payload.utterances) {
    put("presence",row.utterance_id,true); put("text",row.utterance_id,row.effective_text); put("assignment",row.utterance_id,row.speaker_ids);
  }
  for(const row of payload.manual_speakers ?? []) put("manual",row.id,row.label);
  for(const row of payload.speaker_merges ?? []) put("merge",row.source_id,row.target_id);
  for(const row of payload.utterance_splits ?? []) put("split",row.source_utterance_id,row);
  return result;
}

// Suggestions are comparisons, never permission to write. A shared field with
// two different edits requires an explicit choice; unrelated latest changes
// remain intact. Text and speaker assignment are compared independently.
export function planTranscriptConflict(base: DraftPayload,local: DraftPayload,latest: DraftPayload): TranscriptConflictPlan {
  const payloads = [base,local,latest].map(value => {
    if(![1,2,3].includes(value.schema_version)) throw Error("未対応の下書き形式は比較できません");
    return canonicalPayload(value);
  });
  const [before,ours,theirs] = payloads.map(atoms);
  const rows = [...new Set([...before.keys(),...ours.keys(),...theirs.keys()])].sort().flatMap(id => {
    const a = before.get(id)?.value,b = ours.get(id)?.value,c = theirs.get(id)?.value;
    if(equal(b,c)) return [];
    const item = ours.get(id) ?? theirs.get(id) ?? before.get(id)!;
    const conflict = !equal(a,b) && !equal(a,c);
    return [{ key: id,kind: item.kind,id: item.id,base: a,local: b,latest: c,conflict,
      suggested: conflict ? null : equal(a,b) ? "latest" as const : "local" as const }];
  });
  return immutable({ base: payloads[0],local: payloads[1],latest: payloads[2],rows });
}

export function resolveTranscriptConflict(plan: TranscriptConflictPlan,choices: Record<string,ConflictChoice>): DraftPayload {
  const latest = atoms(plan.latest), selected = new Map(latest), local = atoms(plan.local);
  const validKeys = new Set(plan.rows.map(row => row.key));
  if(Object.keys(choices).some(value => !validKeys.has(value) || !["local","latest"].includes(choices[value]))) throw Error("比較していない項目の選択があります");
  for(const row of plan.rows) {
    const choice = choices[row.key] ?? row.suggested;
    if(!choice) throw Error("競合した項目ごとに手元か最新保存版を選んでください");
    const item = (choice === "local" ? local : latest).get(row.key);
    if(item) selected.set(row.key,item); else selected.delete(row.key);
  }
  const result: DraftPayload = { schema_version: Math.max(plan.local.schema_version,plan.latest.schema_version),speaker_names: [],utterances: [] };
  const utteranceIds = new Set<string>();
  for(const item of selected.values()) {
    if(["presence","text","assignment"].includes(item.kind)) { utteranceIds.add(item.id); continue; }
    if(item.kind === "name") result.speaker_names.push({ speaker_id: item.id,display_name: item.value as string | null });
    if(item.kind === "manual") (result.manual_speakers ??= []).push({ id: item.id,label: item.value as string });
    if(item.kind === "merge") (result.speaker_merges ??= []).push({ source_id: item.id,target_id: item.value as string });
    if(item.kind === "split") (result.utterance_splits ??= []).push(structuredClone(item.value) as NonNullable<DraftPayload["utterance_splits"]>[number]);
  }
  for(const id of utteranceIds) {
    const exists = selected.get(key("presence",id)), text = selected.get(key("text",id)), assignment = selected.get(key("assignment",id));
    if(!exists || !text || !assignment) throw Error("発言の保持・本文・話者割り当ての選択が一致しません。項目を再確認してください");
    result.utterances.push({ utterance_id: id,effective_text: text.value as string,speaker_ids: [...assignment.value as string[]] });
  }
  // Dependencies between added speakers, merges and split IDs are checked by
  // the editor/native validators before any resolved payload may be published.
  return canonicalPayload(result);
}
