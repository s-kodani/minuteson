import type { ModelInspectionReport } from "./model-inspection-state";
export type ImportEngine = "whisper.cpp" | "sherpa-onnx" | "llama.cpp";
export type ImportRole = "executable" | "asr" | "segmentation" | "speaker_embedding" | "llm" | "runtime_library" | "vad";
export type ChosenModelFile = { path: string; role: ImportRole };
export type DeclaredModelManifest = {
  schemaVersion: 1; engine: ImportEngine; engineVersion: string; modelName: string;
  modelVersion: string; quantization: string | null; licenseRef: string; target: "macos_aarch64";
  files: { relativePath: string; role: ImportRole; expectedSize: null; expectedHash: null }[];
};
export type GuidedModelSource = { directory: string; manifest: DeclaredModelManifest };
export const importRoles: Record<ImportEngine, { role: ImportRole; label: string }[]> = {
  "whisper.cpp": [{ role: "executable", label: "文字起こしの実行ファイル" }, { role: "asr", label: "文字起こしモデル" }],
  "sherpa-onnx": [{ role: "executable", label: "話者分離の実行ファイル" }, { role: "segmentation", label: "発話区間モデル" }, { role: "speaker_embedding", label: "話者の声の特徴モデル" }],
  "llama.cpp": [{ role: "executable", label: "LLMの実行ファイル" }, { role: "llm", label: "議事録生成モデル（GGUF）" }],
};
const filename = (path: string) => path.split("/").at(-1) ?? "";
export { filename as modelFileName };
function components(path: string): string[] {
  if (!path.startsWith("/") || path.length > 4096 || /[\x00-\x1f\x7f]/.test(path)) throw Error("選択したファイルの絶対パスが不正です。");
  const parts = path.slice(1).split("/");
  if (parts.some(part => !part || part === "." || part === "..")) throw Error("ファイルの実体を選び直してください。");
  return parts;
}
export function guidedModelSource(engine: ImportEngine, selected: ChosenModelFile[], metadata: { name?: string; engineVersion?: string; modelVersion?: string; licenseRef?: string } = {}): GuidedModelSource {
  const required = importRoles[engine];
  if (!required) throw Error("対応していないAIの種類です。");
  const allowed = new Set<ImportRole>([...required.map(item => item.role), "runtime_library", ...(engine === "whisper.cpp" ? ["vad" as const] : [])]);
  if (selected.length > 20 || selected.some(item => !allowed.has(item.role)) ||
      required.some(item => selected.filter(file => file.role === item.role).length !== 1) ||
      selected.filter(item => item.role === "runtime_library").length > 16 || selected.filter(item => item.role === "vad").length > 1) throw Error("必要な実行ファイルとモデルをそれぞれ選んでください。");
  const paths = selected.map(item => components(item.path));
  const parents = paths.map(parts => parts.slice(0, -1));
  let prefix = parents[0].length;
  for (const parts of parents) { prefix = Math.min(prefix, parts.length); while (prefix && parts.slice(0, prefix).join("/") !== parents[0].slice(0, prefix).join("/")) prefix--; }
  // The root is derived, not enumerated: only explicitly selected files enter
  // inspection/copy. Preserve relative runtime-library layout for loader_path.
  const directory = prefix ? `/${parents[0].slice(0, prefix).join("/")}` : "/";
  const relative = paths.map(parts => parts.slice(prefix).join("/"));
  if (new Set(relative.map(path => path.toLowerCase())).size !== relative.length || relative.some(path => path.toLowerCase() === "mon-model.json")) throw Error("同じファイルを複数の役割へ選択できません。");
  const nonExecutable = selected.find(item => required.some(r => r.role === item.role) && item.role !== "executable")!;
  const declared = (value: string | undefined, fallback: string) => value?.trim() || fallback;
  return { directory, manifest: {
    schemaVersion: 1, engine, target: "macos_aarch64", quantization: null,
    engineVersion: declared(metadata.engineVersion, "未申告"), modelVersion: declared(metadata.modelVersion, "未申告"),
    modelName: declared(metadata.name, filename(nonExecutable.path)),
    licenseRef: declared(metadata.licenseRef, "利用者による手動取り込み・配布元の利用条件は別途確認"),
    files: selected.map((item, i) => ({ relativePath: relative[i], role: item.role, expectedSize: null, expectedHash: null })),
  } };
}

export function assertGuidedInspection(report: ModelInspectionReport, source: GuidedModelSource): void {
  const fail = () => { throw Error("検査結果と選択したファイルの構成が一致しません。再検査してください。"); };
  const manifest = report.manifest as unknown as DeclaredModelManifest;
  for (const key of ["schemaVersion", "engine", "engineVersion", "modelName", "modelVersion", "quantization", "licenseRef", "target"] as const) if (manifest?.[key] !== source.manifest[key]) fail();
  if (!Array.isArray(manifest.files) || manifest.files.length !== source.manifest.files.length || !Array.isArray(report.files) || report.files.length !== source.manifest.files.length || !/^[0-9a-f]{64}$/.test(report.manifestHash) || !/^[0-9a-f]{64}$/.test(report.reviewHash)) fail();
  let total = 0;
  source.manifest.files.forEach((expected, index) => {
    const declared = manifest.files[index], actual = report.files[index];
    if (!declared || !actual || declared.relativePath !== expected.relativePath || declared.role !== expected.role || declared.expectedHash !== null || declared.expectedSize !== null || actual.relativePath !== expected.relativePath || actual.role !== expected.role || actual.expectedSizeChecked !== false || actual.expectedHashChecked !== false || !Number.isSafeInteger(actual.actualSize) || actual.actualSize <= 0 || !/^[0-9a-f]{64}$/.test(actual.actualHash)) fail();
    total += actual.actualSize;
  });
  if (!Number.isSafeInteger(total) || report.totalBytes !== total) fail();
}
