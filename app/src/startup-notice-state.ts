export interface StartupNotice {
  id: string; revision: string; kind: "information" | "model_runtime_update";
  title: string; message: string; priority: number; deferred: boolean;
  optionId: string | null; packageId: string | null; recipeHash: string | null;
}
const sha = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{64}$/u.test(v);
export function checkedStartupNotices(value: unknown): StartupNotice[] {
  if (!Array.isArray(value) || value.length > 128) throw Error("起動時案内を確認できません。");
  const ids = new Set<string>();
  for (const n of value as StartupNotice[]) {
    if (!n || typeof n.id !== "string" || !/^[a-z0-9-]{1,128}$/u.test(n.id) || ids.has(n.id) || !sha(n.revision)
      || !["information", "model_runtime_update"].includes(n.kind) || !Number.isSafeInteger(n.priority) || n.priority < 0 || n.priority > 255
      || typeof n.deferred !== "boolean" || [n.title,n.message].some(s => typeof s !== "string" || !s.trim() || s.includes("\0") || s.length > 8192)
      || (n.kind === "information" ? n.optionId !== null || n.packageId !== null || n.recipeHash !== null
        : typeof n.optionId !== "string" || !/^[a-z0-9-]{1,128}$/u.test(n.optionId) || typeof n.packageId !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(n.packageId) || !sha(n.recipeHash))) throw Error("起動時案内の内容が不正です。");
    ids.add(n.id);
  }
  return [...value].sort((a,b) => a.priority - b.priority || a.id.localeCompare(b.id));
}
