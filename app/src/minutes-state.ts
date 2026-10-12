export interface MinutesEngine {
  executablePath: string; modelPath: string; threads: number;
  contextTokens: number; predictionTokens: number; gpuLayers: number; seed: number; promptBudget: number;
}
export interface MinutesPreferences { standard: MinutesEngine; highAccuracy: MinutesEngine }
export function minutesConfigured(engine: MinutesEngine): boolean {
  const integer = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
  return !!engine.executablePath.trim() && !!engine.modelPath.trim() && integer(engine.threads, 1, 64)
    && integer(engine.contextTokens, 2048, 32768) && integer(engine.predictionTokens, 64, 8192)
    && integer(engine.gpuLayers, 0, 256) && integer(engine.seed, 0, 4294967295)
    && integer(engine.promptBudget, 2048, 60000) && engine.promptBudget <= engine.contextTokens - engine.predictionTokens - 512;
}
