import type { AnalysisJob } from "./analysis-state.ts";
import type { ModelFileReport, ModelPlacementEntry } from "./model-inspection-state.ts";
import { canReviewApplication } from "./model-profile-state.ts";

export interface RebindingJob extends AnalysisJob {
  meetingId: string; operationId: string; inputHash: string; createdAt: number; updatedAt: number;
  outputMinutesId: string | null;
}
export interface JobModelPreview {
  schemaVersion: 1; source: RebindingJob; checkpointHash: string; reviewHash: string;
  logicalConditions: {
    profile: "standard" | "high_accuracy"; engine: string; configuration: Record<string, number | boolean>;
    vocabularyHint: { characters: number; sha256: string } | null;
    trackSettings: { trackId: string; numSpeakers: number | null }[] | null;
  };
  binding: {
    schemaVersion: 1;
    package: {
      schemaVersion: 1; packageId: string; sourceManifestHash: string; reviewHash: string;
      manifest: {
        schemaVersion: 1; modelName: string; modelVersion: string; engine: string; engineVersion: string;
        target: string; licenseRef: string; quantization: string | null;
        files: { relativePath: string; role: string; expectedSize: number | null; expectedHash: string | null }[];
      };
      files: ModelFileReport[];
    };
    compatibility: {
      schemaVersion: 1; operationId: string; packageId: string; reviewHash: string; engine: string;
      declaredEngineVersion: string; adapterRevision: number; settings: Record<string, unknown>;
      artifacts: { name: string; size: number; sha256: string }[]; elapsedMs: number;
    };
    files: { path: string; size: number; sha256: string }[];
  };
}
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(v);
const sha = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{64}$/.test(v);
const integer = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const text = (v: unknown): v is string => typeof v === "string" && !!v.trim() && v.length <= 4096 && !/[\x00-\x1f]/.test(v);
const engines: Record<string, string> = { transcription: "whisper.cpp", diarization: "sherpa-onnx", minutes_generation: "llama.cpp" };
const unfinished = ["queued", "running", "pausing", "paused", "interrupted"];
export function canRebindJob(job: AnalysisJob, jobs: AnalysisJob[]): boolean {
  return Object.hasOwn(engines, job.kind) && id(job.id) && sha(job.specHash)
    && (job.state === "failed" || job.state === "cancelled" || (job.state === "interrupted" && job.errorCode === "restored_preparation_required"))
    && Number.isSafeInteger(job.attempt) && job.attempt >= 1 && job.attempt < 1000
    && integer(job.completedUnits) && integer(job.totalUnits) && job.totalUnits >= 1 && job.completedUnits <= job.totalUnits
    && !jobs.some(child => child.retryOfJobId === job.id && unfinished.includes(child.state));
}
export function rebindingEntries(entries: ModelPlacementEntry[], kind: string) {
  if (!Array.isArray(entries)) throw Error("モデル配置記録の応答が不正です。");
  const seen = new Set<string>();
  return entries.filter(entry => {
    if (!entry || !id(entry.packageId) || !sha(entry.reviewHash) || seen.has(entry.packageId)) fail();
    seen.add(entry.packageId);
    return canReviewApplication(entry) && entry.engine === engines[kind];
  });
}
function fail(): never { throw Error("モデル再照合の応答と確認したジョブ・配置証拠が一致しません。再読込し、条件を再確認してください。"); }
function assertJob(job: RebindingJob) {
  if (!job || !id(job.id) || !id(job.meetingId) || !id(job.operationId) || !sha(job.inputHash) || !sha(job.specHash)
    || !Object.hasOwn(engines, job.kind) || ![...unfinished, "succeeded", "failed", "cancelled"].includes(job.state)
    || !integer(job.completedUnits) || !integer(job.totalUnits) || job.totalUnits < 1 || job.completedUnits > job.totalUnits
    || !integer(job.attempt) || job.attempt < 1 || job.attempt > 1000 || !integer(job.createdAt) || !integer(job.updatedAt)
    || [job.outputAnalysisId, job.outputMinutesId, job.retryOfJobId].some(value => value !== null && !id(value))
    || [job.errorCode, job.pauseReason].some(value => value !== null && !text(value))) fail();
}
export function assertJobModelPreview(p: JobModelPreview, meetingId: string, job: AnalysisJob, entry: ModelPlacementEntry) {
  if (!p || p.schemaVersion !== 1 || !sha(p.reviewHash) || !sha(p.checkpointHash) || !canRebindJob(p.source, [])
    || p.source.meetingId !== meetingId || !canReviewApplication(entry)) fail();
  assertJob(p.source);
  for (const key of ["id", "kind", "state", "specHash", "completedUnits", "totalUnits", "attempt", "errorCode", "retryOfJobId"] as const) if (p.source[key] !== job[key]) fail();
  const conditions = p.logicalConditions, b = p.binding, pack = b?.package, m = pack?.manifest, check = b?.compatibility;
  if (!conditions || !["standard", "high_accuracy"].includes(conditions.profile) || conditions.engine !== engines[job.kind]
    || !conditions.configuration || typeof conditions.configuration !== "object" || Array.isArray(conditions.configuration)
    || b?.schemaVersion !== 1 || pack?.schemaVersion !== 1 || m?.schemaVersion !== 1
    || pack.packageId !== entry.packageId || pack.reviewHash !== entry.reviewHash || !sha(pack.sourceManifestHash)
    || m.engine !== conditions.engine || m.engine !== entry.engine || m.modelName !== entry.modelName || m.modelVersion !== entry.modelVersion || m.engineVersion !== entry.engineVersion
    || !text(m.licenseRef) || !["macos_aarch64", "windows_x86_64"].includes(m.target) || (m.quantization !== null && !text(m.quantization))
    || check?.schemaVersion !== 1 || check.adapterRevision !== 1 || check.operationId !== entry.compatibility?.operationId
    || check.packageId !== pack.packageId || check.reviewHash !== pack.reviewHash || check.engine !== m.engine || check.declaredEngineVersion !== m.engineVersion
    || !integer(check.elapsedMs) || !check.settings || typeof check.settings !== "object" || !Array.isArray(check.artifacts) || !check.artifacts.length
    || check.artifacts.some(a => !a || !text(a.name) || !integer(a.size) || !sha(a.sha256))) fail();
  const configurationKeys: Record<string, string[]> = {
    "whisper.cpp": ["threads", "useGpu"],
    "sherpa-onnx": ["threads", "clusterThresholdMilli", "windowShiftMilli", "minDurationOnMs", "minDurationOffMs"],
    "llama.cpp": ["threads", "contextTokens", "predictionTokens", "gpuLayers", "seed", "promptBudget"],
  };
  const keys = configurationKeys[conditions.engine];
  if (Object.keys(conditions.configuration).length !== keys.length || keys.some(k => !Object.hasOwn(conditions.configuration, k)
    || (k === "useGpu" ? typeof conditions.configuration[k] !== "boolean" : !integer(conditions.configuration[k])))
    || Number(conditions.configuration.threads) < 1 || Number(conditions.configuration.threads) > 64) fail();
  if (job.kind === "transcription" ? !conditions.vocabularyHint || !integer(conditions.vocabularyHint.characters) || !sha(conditions.vocabularyHint.sha256) : conditions.vocabularyHint !== null) fail();
  if (job.kind === "diarization") {
    const tracks = conditions.trackSettings;
    if (!Array.isArray(tracks) || !tracks.length || new Set(tracks.map(t => t?.trackId)).size !== tracks.length
      || tracks.some(t => !t || !id(t.trackId) || (t.numSpeakers !== null && (!integer(t.numSpeakers) || t.numSpeakers < 1 || t.numSpeakers > 100)))) fail();
  } else if (conditions.trackSettings !== null) fail();
  if (!Array.isArray(pack.files) || !pack.files.length || !Array.isArray(m.files) || !Array.isArray(b.files)
    || m.files.length !== pack.files.length || b.files.length !== pack.files.length) fail();
  const names = new Set<string>();
  let total = 0;
  pack.files.forEach((f, i) => {
    const declared = m.files[i], proof = b.files[i];
    if (!f || !declared || !proof || !text(f.relativePath) || f.relativePath.startsWith("/") || /\\/.test(f.relativePath)
      || f.relativePath.split("/").some(part => !part || part === "." || part === "..") || names.has(f.relativePath)
      || !["executable", "runtime_library", "asr", "vad", "segmentation", "speaker_embedding", "llm"].includes(f.role)
      || !integer(f.actualSize) || f.actualSize < 1 || !sha(f.actualHash) || declared.relativePath !== f.relativePath || declared.role !== f.role
      || (declared.expectedSize === null ? f.expectedSizeChecked !== false : declared.expectedSize !== f.actualSize || f.expectedSizeChecked !== true)
      || (declared.expectedHash === null ? f.expectedHashChecked !== false : declared.expectedHash !== f.actualHash || f.expectedHashChecked !== true)
      || proof.size !== f.actualSize || proof.sha256 !== f.actualHash || !text(proof.path) || !proof.path.endsWith(`/${pack.packageId}/${f.relativePath}`) || !proof.path.startsWith("/")) fail();
    names.add(f.relativePath); total += f.actualSize;
  });
  if (!Number.isSafeInteger(total) || total !== entry.totalBytes) fail();
}
export function assertJobModelResult(job: RebindingJob, operationId: string, p: JobModelPreview) {
  assertJob(job);
  if (!id(operationId) || job.operationId !== operationId || job.id === p.source.id || job.meetingId !== p.source.meetingId
    || job.kind !== p.source.kind || job.inputHash !== p.source.inputHash || job.retryOfJobId !== p.source.id || job.attempt !== p.source.attempt + 1
    || job.completedUnits < p.source.completedUnits || job.totalUnits !== p.source.totalUnits) fail();
}

// Keep unresolved receipt identity across meeting switches within this session.
// In particular, closing a dialog must not silently generate a new operation.
export class ReviewedModelRetry {
  readonly preview: JobModelPreview;
  readonly operationId: string;
  sent = false;
  cancelled = false;
  constructor(preview: JobModelPreview, operationId: string) {
    if (!id(operationId)) fail();
    this.preview = structuredClone(preview); this.operationId = operationId;
  }
  request(): { token: string; preview: JobModelPreview } {
    this.sent = true;
    return { token: this.operationId, preview: structuredClone(this.preview) };
  }
  receipt(): { operationId: string; preview: JobModelPreview } {
    return { operationId: this.operationId, preview: structuredClone(this.preview) };
  }
}
