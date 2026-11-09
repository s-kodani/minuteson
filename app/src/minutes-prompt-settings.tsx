import { InfoPopover } from "./info-popover";
import { IconButton } from "./action-icon";
import { invoke, isTauri } from "./ipc.ts";
import { useEffect, useId, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { useActionConfirmation } from "./confirmation-service";
import { IssueNotice } from "./toast";
import { promptValidation, resetPrompt, type MinutesPromptPreferences, type MinutesPromptSnapshot } from "./minutes-prompt-state";

const read = () => invoke<MinutesPromptSnapshot>("get_minutes_prompt_preferences");
export function MinutesPromptSettings({ visible, locked }: { visible: boolean; locked: boolean }) {
  const id = useId();
  const composition = useRef<{ done: Promise<void>; finish: () => void } | null>(null);
  const [queue] = useState(() => new DraftSaveQueue<MinutesPromptPreferences, MinutesPromptSnapshot>(
    async (_id, expectedLockVersion, payload) => { await composition.current?.done; return invoke("save_minutes_prompt_preferences", { expectedLockVersion, payload }); },
    payload => ({ ...payload }), read,
  ));
  const [state, setState] = useState(() => queue.snapshot());
  const [loading, setLoading] = useState(false);
  const [readError, setReadError] = useState("");
  const [composing, setComposing] = useState(false);
  const confirm = useActionConfirmation(locked);
  const flush = async () => {
    await composition.current?.done;
    const current = queue.snapshot();
    if (!current.dirty) return;
    const invalid = current.payload && promptValidation(current.payload);
    if (invalid) throw Error(invalid);
    await queue.flush();
  };
  useEffect(() => {
    const unsubscribe = queue.subscribe(() => setState(queue.snapshot()));
    const unregister = registerPendingEdits(flush, () => {
      const current = queue.snapshot();
      return current.dirty || current.saving || !!composition.current;
    });
    return () => { unsubscribe(); unregister(); composition.current?.finish(); };
  }, [queue]);
  const load = async () => {
    setLoading(true); setReadError("");
    try { queue.load(await read()); }
    catch (cause) { setReadError(String(cause)); }
    finally { setLoading(false); }
  };
  useEffect(() => {
    const current = queue.snapshot();
    if (visible && isTauri() && !current.dirty && !current.saving) void load();
  }, [visible, queue]);
  useEffect(() => {
    if (!visible || locked || composing || loading || confirm.pending || !state.dirty || state.saving || state.error || !state.payload || promptValidation(state.payload)) return;
    const timer = window.setTimeout(() => { void flush().catch(() => {}); }, 700);
    return () => window.clearTimeout(timer);
  }, [visible, locked, composing, loading, confirm.pending, state, queue]);
  const reload = async () => {
    const current = queue.snapshot();
    if (!current.dirty) return load();
    const expected = JSON.stringify(current.payload);
    await confirm({ title: "プロンプト入力の破棄を確認", target: "共通の議事録生成プロンプト", confirmLabel: "入力を破棄して再読込する", message: "未保存のプロンプト入力を破棄し、保存済み設定を読み込みます。", validate: () => JSON.stringify(queue.snapshot().payload) === expected, action: load });
  };
  const reset = async (field: "extractPrompt" | "mergePrompt") => {
    const defaults = queue.snapshot().draft?.defaults;
    if (!defaults) return;
    queue.edit(value => resetPrompt(value, defaults, field));
    try { await flush(); } catch { /* queue retains the input and save error */ }
  };
  const disabled = locked || loading || confirm.pending || !state.payload;
  const invalid = state.payload ? promptValidation(state.payload) : "";
  return <section hidden={!visible} className="status-card minutes-prompt-settings" aria-label="議事録生成プロンプト">
    <div className="heading-with-info"><h2>議事録生成プロンプト</h2><InfoPopover label="議事録生成プロンプト">すべての会議・モデルで共通の設定です。編集内容は自動保存され、次の新規生成から使われます。音声の根拠にはアプリ共通の引用IDを使い、保存時に音声リンクへ変換します。</InfoPopover></div>
    {([ ["extractPrompt", "作成用プロンプト", "文字起こしから議事録を作成するときに使います。"], ["mergePrompt", "統合用プロンプト", "長い会議を分割して作成した下書きを、一つの議事録にまとめるときに使います。"] ] as const).map(([field, label, help]) => <div key={field}>
      <div className="heading-with-info"><label htmlFor={`${id}-${field}`} className="prompt-label">{label}</label><InfoPopover label={label}>{help}</InfoPopover></div>
      <div className="prompt-label"><textarea id={`${id}-${field}`} aria-label={label} value={state.payload?.[field] ?? ""} disabled={disabled} rows={10}
        onCompositionStart={() => {
          if (!composition.current) { let finish!: () => void; const done = new Promise<void>(resolve => { finish = resolve; }); composition.current = { done, finish }; }
          setComposing(true);
        }} onCompositionEnd={() => { composition.current?.finish(); composition.current = null; setComposing(false); }}
        onChange={event => { const text = event.target.value; queue.edit(value => ({ ...value, [field]: text })); }} />
      </div>
      <button type="button" className="small-secondary" aria-label={`${label}をデフォルトに戻す`} disabled={disabled || state.saving || composing || state.payload?.[field] === state.draft?.defaults[field]} onClick={() => void reset(field)}>デフォルトに戻す</button>
    </div>)}
    <div className="analysis-actions">
      <span role="status">{loading ? "読込中…" : state.saving ? "保存中…" : state.dirty ? "未保存" : state.draft ? "保存済み" : ""}</span>
      {state.error && <button type="button" disabled={disabled || state.saving || composing || !!invalid} onClick={() => void queue.retry().catch(() => {})}>保存を再試行</button>}
      <IconButton label="議事録プロンプト設定を再読込" icon="refresh" type="button" className="small-secondary" disabled={locked || loading || state.saving || composing || confirm.pending} onClick={() => void reload()} />
    </div>
    <IssueNotice scope="minutes-prompts" type="preferences" label="議事録プロンプト設定" message={readError || invalid || state.error || ""} action={state.error && !invalid ? { kind: "retry", label: "プロンプトの保存を再試行", disabled: disabled || state.saving || composing, run: async () => { await queue.retry(); } } : readError ? { kind: "retry", label: "プロンプト設定を再読込", disabled: locked || loading, run: load } : undefined} />
  </section>;
}
