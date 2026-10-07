export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export type TextSize = 13 | 15 | 18 | 20;
export interface ReadingPreferences { textSize: TextSize; followPlayback: boolean }
export type UiPreferencesPayload = { schemaVersion: 1; theme: ThemePreference } | ({ schemaVersion: 2; theme: ThemePreference } & ReadingPreferences);
export interface UiPreferencesSnapshot { id: "ui_preferences"; payload: UiPreferencesPayload; lockVersion: number }
export interface ThemeNotice { id: number; message: string; level: "success" | "info" }
export interface ThemeState { snapshot: UiPreferencesSnapshot | null; pendingTheme: ThemePreference | null; pendingDisplay: Partial<ReadingPreferences> | null; pendingPreferences: UiPreferencesPayload | null; busy: boolean; uncertain: boolean; error: string; status: string; notice: ThemeNotice | null }
export interface ThemeBridge { read(): Promise<unknown>; save(version: number, theme: ThemePreference): Promise<unknown>; savePayload?(version: number, payload: UiPreferencesPayload): Promise<unknown> }

export function isTheme(value: unknown): value is ThemePreference { return value === "system" || value === "light" || value === "dark"; }
export function isTextSize(value: unknown): value is TextSize { return value === 13 || value === 15 || value === 18 || value === 20; }
export function readingPreferences(payload?: UiPreferencesPayload | null): ReadingPreferences {
  return payload?.schemaVersion === 2 ? { textSize:payload.textSize, followPlayback:payload.followPlayback } : { textSize:15, followPlayback:false };
}
const samePreferences = (a: UiPreferencesPayload, b: UiPreferencesPayload) => JSON.stringify(a) === JSON.stringify(b);
export function resolveTheme(preference: ThemePreference, osDark: boolean): ResolvedTheme { return preference === "system" ? osDark ? "dark" : "light" : preference; }
export function assertUiPreferences(value: unknown): UiPreferencesSnapshot {
  const v = value as UiPreferencesSnapshot;
  if (!v || typeof v !== "object" || Array.isArray(v) || Object.keys(v).sort().join() !== "id,lockVersion,payload" || v.id !== "ui_preferences" || !Number.isSafeInteger(v.lockVersion) || v.lockVersion < 0 || !v.payload || typeof v.payload !== "object" || Array.isArray(v.payload) || !isTheme(v.payload.theme)) throw Error("表示設定の応答が不正です");
  const keys = Object.keys(v.payload).sort().join();
  let payload: UiPreferencesPayload;
  if (v.payload.schemaVersion === 1 && keys === "schemaVersion,theme") payload = { schemaVersion:1,theme:v.payload.theme };
  else if (v.payload.schemaVersion === 2 && keys === "followPlayback,schemaVersion,textSize,theme" && isTextSize(v.payload.textSize) && typeof v.payload.followPlayback === "boolean") payload = { schemaVersion:2,theme:v.payload.theme,textSize:v.payload.textSize,followPlayback:v.payload.followPlayback };
  else throw Error("表示設定の応答が不正です");
  if (v.lockVersion === 0 && (payload.schemaVersion !== 1 || payload.theme !== "system")) throw Error("未保存の表示設定が不正です");
  return { id:"ui_preferences",lockVersion:v.lockVersion,payload };
}

// RPC results are authoritative. Events only request a read. In-flight reads
// cannot overwrite a newer save, and uncertain saves cannot be blindly retried.
export class ThemeController {
  private bridge: ThemeBridge;
  private value: ThemeState = { snapshot: null, pendingTheme: null, pendingDisplay:null, pendingPreferences:null, busy: false, uncertain: false, error: "", status: "", notice: null };
  private listeners = new Set<() => void>();
  private generation = 0;
  private noticeId = 0;
  private reading: Promise<void> | null = null;
  constructor(bridge: ThemeBridge) { this.bridge = bridge; }
  getSnapshot = (): ThemeState => this.value;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ThemeState>) { this.value = { ...this.value, ...patch }; for (const listener of this.listeners) listener(); }
  private notify(message: string, level: ThemeNotice["level"]) { this.update({ notice: { id: ++this.noticeId, message, level } }); }
  clearNotice = (id: number) => { if (this.value.notice?.id === id) this.update({ notice: null }); };
  private requestedMatches(payload: UiPreferencesPayload) {
    if (this.value.pendingTheme !== null) return payload.theme === this.value.pendingTheme;
    const patch = this.value.pendingDisplay;
    return !!patch && payload.schemaVersion === 2 && Object.entries(patch).every(([key,value])=>payload[key as keyof ReadingPreferences] === value);
  }
  private accept(snapshot: UiPreferencesSnapshot) {
    const current = this.value.snapshot;
    if (current && snapshot.lockVersion < current.lockVersion) throw Error("古い表示設定の応答を採用しません");
    if (current && snapshot.lockVersion === current.lockVersion && !samePreferences(snapshot.payload,current.payload)) throw Error("同じ版の表示設定が変更されています");
    this.update({ snapshot, uncertain: false });
  }
  refresh = (): Promise<void> => {
    if (this.value.busy) return Promise.resolve();
    if (this.reading) return this.reading;
    const generation = this.generation;
    this.reading = (async () => {
      try {
        const snapshot = assertUiPreferences(await this.bridge.read());
        if (generation !== this.generation) return;
        this.accept(snapshot);
        if (this.value.pendingPreferences && this.requestedMatches(snapshot.payload)) {
          this.update({ pendingTheme: null, pendingDisplay:null, pendingPreferences:null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        }
        else if (this.value.pendingPreferences === null) this.update({ error: "" });
      } catch (error) {
        if (generation === this.generation) this.update({ uncertain: true, error: `表示設定を確認できません: ${String(error)}` });
      } finally { this.reading = null; }
    })();
    return this.reading;
  };
  reload = async () => {
    if (this.value.busy) return;
    const generation = this.generation;
    this.update({ pendingTheme: null, pendingDisplay:null, pendingPreferences:null, status: "", notice: null });
    await this.refresh();
    if (generation === this.generation && !this.value.busy && !this.value.uncertain && this.value.snapshot) this.notify("保存済みの表示設定を再読込しました。", "info");
  };
  change = async (theme: ThemePreference): Promise<void> => {
    if (!isTheme(theme)) throw Error("テーマが不正です");
    if (this.value.pendingPreferences && this.value.pendingTheme === null) return;
    const base = this.value.snapshot;
    if (!base) return;
    await this.savePreferences({ ...base.payload,theme },theme);
  };
  changeDisplay = async (patch: Partial<ReadingPreferences>): Promise<void> => {
    const keys = Object.keys(patch);
    if (!keys.length || keys.some(key=>key !== "textSize" && key !== "followPlayback") || "textSize" in patch && !isTextSize(patch.textSize) || "followPlayback" in patch && typeof patch.followPlayback !== "boolean") throw Error("文字サイズ・追従設定が不正です");
    const base = this.value.snapshot;
    if (!base || this.value.pendingPreferences) return;
    await this.savePreferences({ schemaVersion:2,theme:base.payload.theme,...readingPreferences(base.payload),...patch },null,{ ...patch });
  };
  retry = async (): Promise<void> => {
    if (this.value.pendingTheme) await this.change(this.value.pendingTheme);
    else if (this.value.pendingDisplay && this.value.snapshot) {
      const base=this.value.snapshot.payload,patch=this.value.pendingDisplay;
      await this.savePreferences({schemaVersion:2,theme:base.theme,...readingPreferences(base),...patch},null,patch);
    }
  };
  private savePreferences = async (payload: UiPreferencesPayload, pendingTheme: ThemePreference | null, pendingDisplay: Partial<ReadingPreferences> | null = null): Promise<void> => {
    const base = this.value.snapshot;
    if (!base || this.value.busy || this.value.uncertain) return;
    this.generation++;
    this.update({ pendingTheme, pendingDisplay, pendingPreferences:payload, busy: true, error: "", status: "表示設定を保存しています。", notice: null });
    try {
      const reply = payload.schemaVersion === 1 ? await this.bridge.save(base.lockVersion,payload.theme)
        : this.bridge.savePayload ? await this.bridge.savePayload(base.lockVersion,payload) : (()=>{throw Error("表示設定の保存経路がありません");})();
      const saved = assertUiPreferences(reply);
      const expected = base.lockVersion + (samePreferences(base.payload,payload) ? 0 : 1);
      if (!samePreferences(saved.payload,payload) || saved.lockVersion !== expected) throw Error("確認した表示設定と保存結果が一致しません");
      this.accept(saved);
      this.update({ pendingTheme: null, pendingDisplay:null, pendingPreferences:null, error: "", status: "表示設定を保存しました。" });
      this.notify("表示設定を保存しました。", "success");
    } catch (error) {
      try {
        this.accept(assertUiPreferences(await this.bridge.read()));
        if (this.value.snapshot && this.requestedMatches(this.value.snapshot.payload)) {
          this.update({ pendingTheme: null, pendingDisplay:null, pendingPreferences:null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        }
        else this.update({ error: `表示設定を保存できませんでした: ${String(error)}`, status: "選択を保持しています。保存済み設定を確認してから再試行できます。" });
      } catch (readError) {
        this.update({ uncertain: true, error: `保存結果を確認できません。再読込するまで再保存しません: ${String(readError)}`, status: "未確認の選択を保持しています。" });
      }
    } finally { this.update({ busy: false }); }
  };
}
