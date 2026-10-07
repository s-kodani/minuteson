export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export interface UiPreferencesSnapshot { id: "ui_preferences"; payload: { schemaVersion: 1; theme: ThemePreference }; lockVersion: number }
export interface ThemeNotice { id: number; message: string; level: "success" | "info" }
export interface ThemeState { snapshot: UiPreferencesSnapshot | null; pendingTheme: ThemePreference | null; busy: boolean; uncertain: boolean; error: string; status: string; notice: ThemeNotice | null }
export interface ThemeBridge { read(): Promise<unknown>; save(version: number, theme: ThemePreference): Promise<unknown> }

export function isTheme(value: unknown): value is ThemePreference { return value === "system" || value === "light" || value === "dark"; }
export function resolveTheme(preference: ThemePreference, osDark: boolean): ResolvedTheme { return preference === "system" ? osDark ? "dark" : "light" : preference; }
export function assertUiPreferences(value: unknown): UiPreferencesSnapshot {
  const v = value as UiPreferencesSnapshot;
  if (!v || typeof v !== "object" || Array.isArray(v) || Object.keys(v).sort().join() !== "id,lockVersion,payload" || v.id !== "ui_preferences" || !Number.isSafeInteger(v.lockVersion) || v.lockVersion < 0 || !v.payload || typeof v.payload !== "object" || Array.isArray(v.payload) || Object.keys(v.payload).sort().join() !== "schemaVersion,theme" || v.payload.schemaVersion !== 1 || !isTheme(v.payload.theme) || (v.lockVersion === 0 && v.payload.theme !== "system")) throw Error("表示設定の応答が不正です");
  return { id: "ui_preferences", lockVersion: v.lockVersion, payload: { schemaVersion: 1, theme: v.payload.theme } };
}

// RPC results are authoritative. Events only request a read. In-flight reads
// cannot overwrite a newer save, and uncertain saves cannot be blindly retried.
export class ThemeController {
  private bridge: ThemeBridge;
  private value: ThemeState = { snapshot: null, pendingTheme: null, busy: false, uncertain: false, error: "", status: "", notice: null };
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
  private accept(snapshot: UiPreferencesSnapshot) {
    const current = this.value.snapshot;
    if (current && snapshot.lockVersion < current.lockVersion) throw Error("古い表示設定の応答を採用しません");
    if (current && snapshot.lockVersion === current.lockVersion && snapshot.payload.theme !== current.payload.theme) throw Error("同じ版の表示設定が変更されています");
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
        if (this.value.pendingTheme === snapshot.payload.theme) {
          this.update({ pendingTheme: null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        }
        else if (this.value.pendingTheme === null) this.update({ error: "" });
      } catch (error) {
        if (generation === this.generation) this.update({ uncertain: true, error: `表示設定を確認できません: ${String(error)}` });
      } finally { this.reading = null; }
    })();
    return this.reading;
  };
  reload = async () => {
    if (this.value.busy) return;
    const generation = this.generation;
    this.update({ pendingTheme: null, status: "", notice: null });
    await this.refresh();
    if (generation === this.generation && !this.value.busy && !this.value.uncertain && this.value.snapshot) this.notify("保存済みの表示設定を再読込しました。", "info");
  };
  change = async (theme: ThemePreference): Promise<void> => {
    if (!isTheme(theme)) throw Error("テーマが不正です");
    const base = this.value.snapshot;
    if (!base || this.value.busy || this.value.uncertain) return;
    this.generation++;
    this.update({ pendingTheme: theme, busy: true, error: "", status: "表示設定を保存しています。", notice: null });
    try {
      const saved = assertUiPreferences(await this.bridge.save(base.lockVersion, theme));
      const expected = base.lockVersion + (base.payload.theme === theme ? 0 : 1);
      if (saved.payload.theme !== theme || saved.lockVersion !== expected) throw Error("確認したテーマと保存結果が一致しません");
      this.accept(saved);
      this.update({ pendingTheme: null, error: "", status: "表示設定を保存しました。" });
      this.notify("表示設定を保存しました。", "success");
    } catch (error) {
      try {
        this.accept(assertUiPreferences(await this.bridge.read()));
        if (this.value.snapshot?.payload.theme === theme) {
          this.update({ pendingTheme: null, error: "", status: "保存済みの表示設定を確認しました。" });
          this.notify("保存済みの表示設定を確認しました。", "info");
        }
        else this.update({ error: `表示設定を保存できませんでした: ${String(error)}`, status: "選択を保持しています。保存済み設定を確認してから再試行できます。" });
      } catch (readError) {
        this.update({ uncertain: true, error: `保存結果を確認できません。再読込するまで再保存しません: ${String(readError)}`, status: "未確認の選択を保持しています。" });
      }
    } finally { this.update({ busy: false }); }
  };
}
