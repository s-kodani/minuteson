export interface DiarizationEngine {
  executablePath: string; segmentationModelPath: string; embeddingModelPath: string;
  runtimeLibraryPaths: string[]; threads: number; clusterThresholdMilli: number;
  windowShiftMilli: number; minDurationOnMs: number; minDurationOffMs: number;
}
export interface DiarizationPreferences { standard: DiarizationEngine; highAccuracy: DiarizationEngine }
const bounded = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
const pathOK = (s: string) => s.length <= 4096 && !/[\r\n\0]/.test(s) && (/^\//.test(s) || /^[a-z]:[\\/]/i.test(s) || /^\\\\/.test(s));
export function diarizationConfigured(e: DiarizationEngine): boolean {
  return [e.executablePath,e.segmentationModelPath,e.embeddingModelPath].every(pathOK)
    && e.runtimeLibraryPaths.length <= 16 && e.runtimeLibraryPaths.every(pathOK)
    && bounded(e.threads,1,64) && bounded(e.clusterThresholdMilli,0,2000) && bounded(e.windowShiftMilli,1,1000)
    && bounded(e.minDurationOnMs,0,60000) && bounded(e.minDurationOffMs,0,60000);
}
