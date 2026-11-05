export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export type ColorTheme = "monochrome" | "purple";
export type UiPreferencesPayload = { schemaVersion: 1; theme: ThemePreference; colorTheme: ColorTheme };
export interface UiPreferencesSnapshot { id: "ui_preferences"; payload: UiPreferencesPayload; lockVersion: number }
export interface ThemeNotice { id: number; message: string; level: "success" | "info" }
export interface ThemeState { snapshot: UiPreferencesSnapshot | null; pendingTheme: ThemePreference | null; pendingPreferences: UiPreferencesPayload | null; busy: boolean; uncertain: boolean; error: string; status: string; notice: ThemeNotice | null }
export interface ThemeBridge { read(): Promise<unknown>; save(version: number, theme: ThemePreference, colorTheme: ColorTheme): Promise<unknown> }

export function isTheme(value: unknown): value is ThemePreference { return value === "system" || value === "light" || value === "dark"; }
export function isColorTheme(value: unknown): value is ColorTheme { return value === "monochrome" || value === "purple"; }
const samePreferences = (a: UiPreferencesPayload, b: UiPreferencesPayload) => JSON.stringify(a) === JSON.stringify(b);
export function resolveTheme(preference: ThemePreference, osDark: boolean): ResolvedTheme { return preference === "system" ? osDark ? "dark" : "light" : preference; }
export function assertUiPreferences(value: unknown): UiPreferencesSnapshot {
  const v = value as UiPreferencesSnapshot;
  if (!v || typeof v !== "object" || Array.isArray(v) || Object.keys(v).sort().join() !== "id,lockVersion,payload" || v.id !== "ui_preferences" || !Number.isSafeInteger(v.lockVersion) || v.lockVersion < 0 || !v.payload || typeof v.payload !== "object" || Array.isArray(v.payload) || !isTheme(v.payload.theme)) throw Error("表示設定の応答が不正です");
  const keys = Object.keys(v.payload).sort().join();
  let payload: UiPreferencesPayload;
  if (v.payload.schemaVersion === 1 && (keys === "schemaVersion,theme" || keys === "colorTheme,schemaVersion,theme" && isColorTheme(v.payload.colorTheme))) payload = { schemaVersion:1,theme:v.payload.theme,colorTheme:v.payload.colorTheme ?? "monochrome" };
  else throw Error("表示設定の応答が不正です");
  if (v.lockVersion === 0 && (payload.theme !== "system" || payload.colorTheme !== "monochrome")) throw Error("未保存の表示設定が不正です");
  return { id:"ui_preferences",lockVersion:v.lockVersion,payload };
}

// RPC results are authoritative. Events only request a read. In-flight reads
// cannot overwrite a newer save, and uncertain saves cannot be blindly retried.
export class ThemeController {
  private bridge: ThemeBridge;
  private value: ThemeState = { snapshot: null, pendingTheme: null, pendingPreferences:null, busy: false, uncertain: false, error: "", status: "", notice: null };
  private listeners = new Set<() => void>();
  private generation = 0;
  private noticeId = 0;
  private reading: Promise<void> | null = null;
  private pendingChanges: Partial<Pick<UiPreferencesPayload, "theme" | "colorTheme">> | null = null;
  constructor(bridge: ThemeBridge) { this.bridge = bridge; }
  getSnapshot = (): ThemeState => this.value;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ThemeState>) { this.value = { ...this.value, ...patch }; for (const listener of this.listeners) listener(); }
  private notify(message: string, level: ThemeNotice["level"]) { this.update({ notice: { id: ++this.noticeId, message, level } }); }
  clearNotice = (id: number) => { if (this.value.notice?.id === id) this.update({ notice: null }); };
  private requestedMatches(payload: UiPreferencesPayload) {
    return this.value.pendingPreferences !== null && samePreferences(payload, this.value.pendingPreferences);
  }
  private markSaveUnconfirmed(cause: unknown) {
    this.update({ uncertain: true, error: `保存結果を確認できません。再読込するまで再保存しません: ${String(cause)}`, status: "未確認の選択を保持しています。" });
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
        const wasUncertain = this.value.uncertain;
        this.accept(snapshot);
        if (this.requestedMatches(snapshot.payload)) {
          this.pendingChanges = null;
          this.update({ pendingTheme: null, pendingPreferences:null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        } else if (this.value.pendingPreferences === null) {
          this.update({ error: "" });
        } else if (wasUncertain) {
          this.update({ error: "保存済みの表示設定が選択内容と一致しません。保存を再試行できます。", status: "選択を保持しています。" });
        }
      } catch (error) {
        if (generation !== this.generation) return;
        if (this.value.pendingPreferences) {
          // Keep the original save notice while confirmation remains unavailable.
          if (!this.value.uncertain) this.markSaveUnconfirmed(error);
        } else {
          this.update({ uncertain: true, error: `表示設定を確認できません: ${String(error)}` });
        }
      } finally { this.reading = null; }
    })();
    return this.reading;
  };
  reload = async () => {
    if (this.value.busy) return;
    const generation = this.generation;
    this.pendingChanges = null;
    this.update({ pendingTheme: null, pendingPreferences:null, status: "", notice: null });
    await this.refresh();
    if (generation === this.generation && !this.value.busy && !this.value.uncertain && this.value.snapshot) this.notify("保存済みの表示設定を再読込しました。", "info");
  };
  change = async (theme: ThemePreference): Promise<void> => {
    if (!isTheme(theme)) throw Error("テーマが不正です");
    await this.saveChanges({ theme });
  };
  changeColor = async (colorTheme: ColorTheme): Promise<void> => {
    if (!isColorTheme(colorTheme)) throw Error("カラーが不正です");
    await this.saveChanges({ colorTheme });
  };
  retry = async (): Promise<void> => {
    if (this.pendingChanges) await this.saveChanges(this.pendingChanges);
  };
  private saveChanges = async (changes: Partial<Pick<UiPreferencesPayload, "theme" | "colorTheme">>): Promise<void> => {
    const base = this.value.snapshot;
    if (!base || this.value.busy || this.value.uncertain) return;
    this.pendingChanges = { ...this.pendingChanges, ...changes };
    const payload = { ...base.payload, ...this.pendingChanges };
    this.generation++;
    this.update({ pendingTheme:payload.theme, pendingPreferences:payload, busy: true, error: "", status: "表示設定を保存しています。", notice: null });
    try {
      const reply = await this.bridge.save(base.lockVersion,payload.theme,payload.colorTheme);
      const saved = assertUiPreferences(reply);
      const expected = base.lockVersion + (samePreferences(base.payload,payload) ? 0 : 1);
      if (!samePreferences(saved.payload,payload) || saved.lockVersion !== expected) throw Error("確認した表示設定と保存結果が一致しません");
      this.accept(saved);
      this.pendingChanges = null;
      this.update({ pendingTheme: null, pendingPreferences:null, error: "", status: "表示設定を保存しました。" });
      this.notify("表示設定を保存しました。", "success");
    } catch (error) {
      try {
        this.accept(assertUiPreferences(await this.bridge.read()));
        if (this.value.snapshot && this.requestedMatches(this.value.snapshot.payload)) {
          this.pendingChanges = null;
          this.update({ pendingTheme: null, pendingPreferences:null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        }
        else this.update({ error: `表示設定を保存できませんでした: ${String(error)}`, status: "選択を保持しています。保存済み設定を確認してから再試行できます。" });
      } catch (readError) {
        this.markSaveUnconfirmed(readError);
      }
    } finally { this.update({ busy: false }); }
  };
}
