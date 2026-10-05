import type { ModelPlacementEntry } from "../src/model-inspection-state.ts";
import type { JobModelPreview, RebindingJob } from "../src/model-rebinding-state.ts";

export const meetingId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
export const operationId = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
export function rebindingFixture(kind = "transcription") {
  const engine = ({ transcription: "whisper.cpp", diarization: "sherpa-onnx", minutes_generation: "llama.cpp" } as Record<string, string>)[kind];
  const prefix = ({ transcription: "1", diarization: "2", minutes_generation: "3" } as Record<string, string>)[kind];
  const packageId = `${prefix}1111111-1111-4111-8111-111111111111`, checkId = `${prefix}2222222-2222-4222-8222-222222222222`;
  const job: RebindingJob = {
    id: `${prefix}3333333-3333-4333-8333-333333333333`, meetingId, operationId: `${prefix}4444444-4444-4444-8444-444444444444`, kind,
    state: "interrupted", errorCode: "restored_preparation_required", inputHash: "a".repeat(64), specHash: "b".repeat(64), completedUnits: 1, totalUnits: 3,
    outputAnalysisId: null, outputMinutesId: null, pauseReason: null, retryOfJobId: null, attempt: 1, createdAt: 1791160000000, updatedAt: 1791160000000,
  };
  const roles = ["executable", ...(kind === "transcription" ? ["asr"] : kind === "diarization" ? ["segmentation", "speaker_embedding"] : ["llm"]), "runtime_library"];
  const files = roles.map((role, i) => ({ relativePath: `${role}.bin`, role, actualSize: i + 1, actualHash: "c".repeat(64), expectedSizeChecked: true, expectedHashChecked: true }));
  const entry: ModelPlacementEntry = {
    packageId, modelName: `${engine} 確認用モデル`, modelVersion: "fixture-1", engine, engineVersion: "fixture-declared", totalBytes: files.reduce((n, f) => n + f.actualSize, 0),
    placementRecorded: true, ready: false, reviewHash: "d".repeat(64),
    compatibility: { operationId: checkId, packageId, reviewHash: "d".repeat(64), adapterRevision: 1, state: "succeeded", errorMessage: null },
  };
  const configuration = kind === "transcription" ? { threads: 7, useGpu: true }
    : kind === "diarization" ? { threads: 6, clusterThresholdMilli: 700, windowShiftMilli: 400, minDurationOnMs: 200, minDurationOffMs: 600 }
    : { threads: 5, contextTokens: 8192, predictionTokens: 512, gpuLayers: 2, seed: 777, promptBudget: 5000 };
  const preview: JobModelPreview = {
    schemaVersion: 1, source: structuredClone(job), checkpointHash: "e".repeat(64), reviewHash: "f".repeat(64),
    logicalConditions: { profile: "high_accuracy", engine, configuration: configuration as Record<string, number | boolean>,
      vocabularyHint: kind === "transcription" ? { characters: 3, sha256: "a".repeat(64) } : null,
      trackSettings: kind === "diarization" ? [{ trackId: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", numSpeakers: 3 }, { trackId: "cccccccc-cccc-4ccc-8ccc-cccccccccccc", numSpeakers: null }] : null },
    binding: { schemaVersion: 1,
      package: { schemaVersion: 1, packageId, sourceManifestHash: "b".repeat(64), reviewHash: entry.reviewHash, files,
        manifest: { schemaVersion: 1, modelName: entry.modelName, modelVersion: entry.modelVersion, engine, engineVersion: entry.engineVersion,
          target: "macos_aarch64", licenseRef: "検証fixture専用・利用権未確認", quantization: null,
          files: files.map(f => ({ relativePath: f.relativePath, role: f.role, expectedSize: f.actualSize, expectedHash: f.actualHash })) } },
      compatibility: { schemaVersion: 1, operationId: checkId, packageId, reviewHash: entry.reviewHash, engine, declaredEngineVersion: entry.engineVersion,
        adapterRevision: 1, settings: { threads: 1 }, artifacts: [{ name: "result.json", size: 2, sha256: "c".repeat(64) }], elapsedMs: 100 },
      files: files.map(f => ({ path: `/app/models/${packageId}/${f.relativePath}`, size: f.actualSize, sha256: f.actualHash })) },
  };
  const result: RebindingJob = { ...structuredClone(job), id: `${prefix}5555555-5555-4555-8555-555555555555`, operationId, state: "queued", errorCode: null, specHash: "1".repeat(64), attempt: 2, retryOfJobId: job.id };
  return { job, entry, preview, result };
}
