import { planTranscriptConflict, resolveTranscriptConflict, type ConflictChoice, type TranscriptConflictPlan } from "./transcript-conflict.ts";
import { activeSpeakers, canonicalPayload, projectTranscript, type Analysis, type DraftPayload } from "./transcript-state.ts";
import { textEditBaseline, type TextEditSession } from "./transcript-edit-session.ts";
import { nameBaseline, type SpeakerNameSession } from "./speaker-name-edit.ts";
import type { TranscriptRecovery } from "./transcript-recovery-save.ts";

export type InputChoice = "local" | "stored" | "omit";
export interface ConflictInputRow {
  key: string; kind: "text" | "name"; id: string;
  local: TextEditSession | SpeakerNameSession | null;
  stored: TextEditSession | SpeakerNameSession | null;
}
export interface ConflictInputPlan { body: TranscriptConflictPlan; inputs: ConflictInputRow[] }
const same = (a: unknown,b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const uuid = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const length = (value: string) => Array.from(value).length;

// Sparse overrides are expanded only for affected IDs. A carried-over text or
// assignment equal to the original is not a second concurrent modification.
function effective(base: Analysis,payload: DraftPayload,ids: Set<string>) {
  const rows = new Map(base.utterances.map(u => [u.id,{ utterance_id: u.id,effective_text: u.effectiveText,speaker_ids: u.speakerIds }]));
  for(const split of payload.utterance_splits ?? []) for(const part of split.parts) rows.set(part.id,{ utterance_id: part.id,effective_text: part.effective_text,speaker_ids: part.speaker_ids });
  for(const row of payload.utterances) rows.set(row.utterance_id,row);
  // A null name is an explicit override, not absence of an override.
  return canonicalPayload({ ...payload,utterances: [...ids].flatMap(id => rows.has(id) ? [rows.get(id)!] : []) });
}
export function planConflictInput(base: Analysis,before: DraftPayload,local: DraftPayload,latest: DraftPayload,texts: TextEditSession[],names: SpeakerNameSession[],record: TranscriptRecovery<DraftPayload> | null): ConflictInputPlan {
  const payloads = [before,local,latest];
  // Reject duplicates before Map expansion could hide them.
  for(const p of payloads) planTranscriptConflict(p,p,p);
  const ids = new Set(payloads.flatMap(p => p.utterances.map(u => u.utterance_id)));
  const body = planTranscriptConflict(...payloads.map(p => effective(base,p,ids)) as [DraftPayload,DraftPayload,DraftPayload]);
  const inputs: ConflictInputRow[] = [];
  const collect = (kind: "text" | "name",ours: (TextEditSession | SpeakerNameSession)[],stored: (TextEditSession | SpeakerNameSession)[]) => {
    const id = (v: TextEditSession | SpeakerNameSession) => "utteranceId" in v ? v.utteranceId : v.speakerId;
    if(new Set(ours.map(id)).size !== ours.length || new Set(stored.map(id)).size !== stored.length) throw Error("未確定入力に重複したIDがあります");
    const a = new Map(ours.map(v => [id(v),v])), b = new Map(stored.map(v => [id(v),v]));
    for(const key of [...new Set([...a.keys(),...b.keys()])].sort()) inputs.push({ key: JSON.stringify([kind,key]),kind,id: key,local: a.get(key) ?? null,stored: b.get(key) ?? null });
  };
  collect("text",texts,record?.payload.buffers ?? []); collect("name",names,record?.payload.nameBuffers ?? []);
  const plan = structuredClone({ body,inputs });
  const freeze = (v: unknown) => { if(v && typeof v === "object") { Object.values(v).forEach(freeze); Object.freeze(v); } };
  freeze(plan); return plan;
}

// Frontend dependency preflight, not a replacement for native lineage/DB/CAS
// validation. No publishing occurs here or when selecting input to resume.
export function validateConflictPayload(base: Analysis,payload: DraftPayload) {
  if(![1,2,3].includes(payload.schema_version) || payload.schema_version === 1 && ((payload.manual_speakers?.length ?? 0) || (payload.speaker_merges?.length ?? 0))
    || payload.schema_version < 3 && payload.utterance_splits?.length) throw Error("下書き形式と構造の選択が一致しません");
  const speakers = new Set(base.speakers.map(s => s.id));
  if(speakers.size + (payload.manual_speakers?.length ?? 0) > 100) throw Error("話者は推定元を含め100件までです");
  for(const s of payload.manual_speakers ?? []) {
    if(!new RegExp(`^manual:${uuid}$`).test(s.id) || speakers.has(s.id) || !s.label.trim() || length(s.label) > 200) throw Error("追加話者のID・ラベルの選択が不正です");
    speakers.add(s.id);
  }
  const merged = new Set<string>();
  for(const m of payload.speaker_merges ?? []) {
    if(!speakers.has(m.source_id) || !speakers.has(m.target_id) || m.source_id === m.target_id || merged.has(m.source_id)) throw Error("話者統合の参照が一致しません");
    merged.add(m.source_id);
  }
  if(payload.speaker_merges?.some(m => merged.has(m.target_id))) throw Error("話者統合の循環・多段参照を解消してください");
  const assignments = (ids: string[]) => { if(new Set(ids).size !== ids.length || ids.some(id => !speakers.has(id))) throw Error("話者割り当てと追加話者の選択が一致しません"); };
  const original = new Map(base.utterances.map(u => [u.id,u])), utterances = new Set(original.keys()), sources = new Set<string>();
  const overrides = new Map(payload.utterances.map(u => [u.utterance_id,u]));
  if(overrides.size !== payload.utterances.length) throw Error("発言IDが重複しています");
  let count = base.utterances.length;
  for(const split of payload.utterance_splits ?? []) {
    const source = original.get(split.source_utterance_id);
    if(!source || sources.has(source.id) || split.parts.length < 2 || split.parts.length > 1000) throw Error("分割元・分割数の選択が一致しません");
    sources.add(source.id);
    if(split.source_effective_text !== (overrides.get(source.id)?.effective_text ?? source.effectiveText) || length(split.source_effective_text) > 20_000) throw Error("分割元の本文と分割の選択が一致しません");
    let cursor = source.startUs;
    for(const p of split.parts) {
      if(!new RegExp(`^split:${uuid}$`).test(p.id) || utterances.has(p.id) || !Number.isSafeInteger(p.start_us) || !Number.isSafeInteger(p.end_us)
        || p.start_us !== cursor || p.end_us <= cursor || p.end_us > source.endUs || !p.effective_text || length(p.effective_text) > 20_000) throw Error("分割先のID・時刻・本文の選択が一致しません");
      utterances.add(p.id); assignments(p.speaker_ids); cursor = p.end_us;
    }
    if(cursor !== source.endUs || split.parts.map(p => p.effective_text).join("") !== split.source_effective_text) throw Error("分割による本文・時刻の欠落があります");
    count += split.parts.length - 1;
    if(count > 100_000) throw Error("分割後の発言は100000件までです");
  }
  const named = new Set<string>();
  for(const s of payload.speaker_names) {
    if(!speakers.has(s.speaker_id) || named.has(s.speaker_id) || s.display_name !== null && length(s.display_name) > 200) throw Error("話者名の参照・上限の選択が不正です");
    named.add(s.speaker_id);
  }
  for(const u of payload.utterances) {
    if(!utterances.has(u.utterance_id) || length(u.effective_text) > 20_000) throw Error("本文の参照・上限の選択が不正です");
    assignments(u.speaker_ids);
  }
}

export function resolveConflictInput(base: Analysis,plan: ConflictInputPlan,choices: Record<string,ConflictChoice>,inputChoices: Record<string,InputChoice>) {
  let payload = resolveTranscriptConflict(plan.body,choices);
  const defaults = effective(base,{ ...payload,utterances: [] },new Set(payload.utterances.map(u => u.utterance_id)));
  const originalRows = new Map(defaults.utterances.map(u => [u.utterance_id,u]));
  payload = canonicalPayload({ ...payload,utterances: payload.utterances.filter(u => !same(u,originalRows.get(u.utterance_id))) });
  validateConflictPayload(base,payload);
  if(Object.keys(inputChoices).some(key => !plan.inputs.some(row => row.key === key))) throw Error("比較していない入力の選択があります");
  const view = projectTranscript(base,payload), visible = new Map(view.utterances.map(u => [u.id,u])), speakers = new Map(activeSpeakers(view,payload).map(s => [s.id,s]));
  const texts: TextEditSession[] = [], names: SpeakerNameSession[] = [];
  for(const row of plan.inputs) {
    const choice = inputChoices[row.key];
    if(!["local","stored","omit"].includes(choice)) throw Error("未確定入力ごとに再開する値か再開しないかを選んでください");
    if(choice === "omit") continue;
    const session = row[choice];
    if(!session) throw Error("選択した側に入力がありません");
    if(row.kind === "text") {
      const u = visible.get(row.id);
      if(!u) throw Error("選択した発言は現在の構造にありません。部分的に再開せず入力を保持しています");
      texts.push({ ...textEditBaseline(payload,u),value: session.value });
    } else {
      const s = speakers.get(row.id);
      if(!s) throw Error("選択した話者は現在の構造にありません。部分的に再開せず入力を保持しています");
      names.push({ ...nameBaseline(payload,s),value: session.value });
    }
  }
  return { payload,texts,names };
}
