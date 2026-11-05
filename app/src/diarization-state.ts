export interface DiarizationEngine {
  executablePath: string; segmentationModelPath: string; embeddingModelPath: string;
  runtimeLibraryPaths: string[]; threads: number; clusterThresholdMilli: number;
  windowShiftMilli: number; minDurationOnMs: number; minDurationOffMs: number;
}
export interface DiarizationPreferences { standard: DiarizationEngine; highAccuracy: DiarizationEngine }
