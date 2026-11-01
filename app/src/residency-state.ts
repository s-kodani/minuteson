export interface ResidencyState { supported: boolean; canBackground: boolean; trayAvailable: boolean; dockAvailable: boolean; error: string | null }
export function checkedResidencyState(input: unknown): ResidencyState {
  const value = input as ResidencyState;
  if (!value || Object.keys(value).sort().join() !== "canBackground,dockAvailable,error,supported,trayAvailable" || [value.supported, value.canBackground, value.trayAvailable, value.dockAvailable].some(flag => typeof flag !== "boolean") || value.error !== null && (typeof value.error !== "string" || value.error.includes("\0") || value.error.length > 16384) || value.canBackground && (!value.supported || !value.trayAvailable && !value.dockAvailable)) throw Error("アプリの画面を開き直せることを確認できません。画面を表示したままにします。");
  return { ...value };
}
