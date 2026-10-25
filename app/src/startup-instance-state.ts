export interface StartupInstanceReport {
  schemaVersion: 1;
  supported: boolean;
  targetId: string | null;
  canRestart: boolean;
  message: string;
}
export function checkedStartupInstance(input: unknown): StartupInstanceReport {
  const value = input as StartupInstanceReport;
  if (!value || Object.keys(value).sort().join() !== "canRestart,message,schemaVersion,supported,targetId" || value.schemaVersion !== 1 || typeof value.supported !== "boolean" || typeof value.canRestart !== "boolean" || typeof value.message !== "string" || value.message.includes("\0") || new TextEncoder().encode(value.message).length > 4096 || (value.targetId !== null && (typeof value.targetId !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value.targetId))) || value.canRestart !== (value.targetId !== null) || !value.supported && value.canRestart) throw Error("復旧対象を確認できません。終了操作は行っていません。");
  return { ...value };
}
