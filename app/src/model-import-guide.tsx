import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { InfoPopover } from "./info-popover";
import { topOpenModal } from "./modal-state";
import { guidedModelSource, importRoles, modelFileName, type ChosenModelFile, type GuidedModelSource, type ImportEngine, type ImportRole } from "./model-import-guide-state";

export function ModelImportGuide({ disabled, onInspect, onChanged, onPickingChange }: { disabled: boolean; onInspect: (source: GuidedModelSource) => Promise<void>; onChanged: () => void; onPickingChange: (picking: boolean) => void }) {
  const [engine, setEngine] = useState<ImportEngine>("whisper.cpp");
  const [files, setFiles] = useState<ChosenModelFile[]>([]);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState("");
  const current = useRef({ disabled, engine, picking }); current.current = { disabled, engine, picking };
  const mounted = useRef(true), sequence = useRef(0), selecting = useRef(false);
  const focusReturn = useRef<{ element: HTMLButtonElement; request: number } | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; sequence.current++; }; }, []);
  useLayoutEffect(() => {
    if (picking || disabled || !focusReturn.current) return;
    const pending = focusReturn.current, { element, request } = pending;
    const restore = () => {
      if (!(mounted.current && request === sequence.current && !current.current.disabled && element.isConnected && !element.disabled && topOpenModal()?.contains(element)) || !element.getClientRects().length || element.closest("[hidden], [inert]")) return;
      const active = document.activeElement, modal = topOpenModal();
      // Respect a field the user chose while the native panel was returning.
      if (active !== element && active !== document.body && active !== modal && active instanceof HTMLElement && active.isConnected && active.getClientRects().length && !active.matches(":disabled") && !active.closest("[hidden], [inert]")) return;
      element.focus({ preventScroll: true });
    };
    // Re-enable in the commit first. macOS may restore its field editor after
    // the IPC response, so make one bounded, lost-focus-only follow-up too.
    restore();
    const frame = requestAnimationFrame(restore);
    const timeout = window.setTimeout(() => { restore(); if (focusReturn.current === pending) focusReturn.current = null; }, 150);
    return () => { cancelAnimationFrame(frame); window.clearTimeout(timeout); };
  }, [picking, disabled]);
  const select = async (role: ImportRole, element: HTMLButtonElement) => {
    if (current.current.disabled || selecting.current || !isTauri()) return;
    selecting.current = true;
    element.focus({ preventScroll: true });
    const request = ++sequence.current, selectedEngine = engine;
    setPicking(true); onPickingChange(true); setError("");
    try {
      const result = await invoke<string[] | null>("select_model_files", { multiple: role === "runtime_library" });
      if (!mounted.current || request !== sequence.current || current.current.disabled || current.current.engine !== selectedEngine || result === null) return;
      if (!Array.isArray(result) || !result.length || result.length > (role === "runtime_library" ? 16 : 1) || result.some(path => typeof path !== "string" || !path.startsWith("/"))) throw Error("ファイル選択の結果を確認できません。選び直してください。");
      setFiles(previous => [...previous.filter(file => file.role !== role), ...result.map(path => ({ path, role }))]);
      onChanged();
    } catch (cause) { if (mounted.current && request === sequence.current) setError(String(cause)); }
    finally {
      selecting.current = false;
      if (mounted.current && request === sequence.current) {
        focusReturn.current = { element, request };
        setPicking(false); onPickingChange(false);
      }
    }
  };
  const ready = importRoles[engine].every(item => files.some(file => file.role === item.role));
  const blocked = disabled || picking || !isTauri();
  return <section className="model-import-guide" aria-label="ファイルを選んでモデルを取り込む">
    <div className="heading-with-info"><p>実行ファイルとモデルを選んで取り込みます。</p><InfoPopover label="ファイルからのモデル取り込み">JSONの作成や元ファイルの移動は不要です。macOS Apple Silicon用のCLI実行ファイルを選んでください。配布元・利用条件は別途確認が必要です。ファイル名をモデル名とし、版は「未申告」で記録します。性能・精度は未評価です。</InfoPopover></div>
    <label className="prompt-label">取り込むAI<select value={engine} disabled={blocked} onChange={event => { sequence.current++; setEngine(event.target.value as ImportEngine); setFiles([]); setError(""); onChanged(); }}>
      <option value="whisper.cpp">文字起こし（whisper.cpp）</option><option value="sherpa-onnx">話者分離（sherpa-onnx）</option><option value="llama.cpp">議事録生成（llama.cpp）</option>
    </select></label>
    <div className="model-import-file-list">{importRoles[engine].map(item => <div className="model-import-file" key={item.role}><span>{item.label}<small title={files.find(file => file.role === item.role)?.path}>{modelFileName(files.find(file => file.role === item.role)?.path ?? "") || "未選択"}</small></span><button type="button" className="small-secondary" disabled={blocked} onClick={event => void select(item.role, event.currentTarget)} aria-label={`${item.label}を選択`}>選択</button></div>)}</div>
    <details><summary>追加の依存ファイル</summary><p>配布元の構成に含まれる共有ライブラリがある場合は、元の配置関係を保って選択してください。アプリによる自動検出は行いません。</p><button type="button" className="small-secondary" disabled={blocked} onClick={event => void select("runtime_library", event.currentTarget)}>共有ライブラリを選択</button>{files.filter(file => file.role === "runtime_library").map(file => <p key={file.path} title={file.path}>{modelFileName(file.path)}</p>)}{engine === "whisper.cpp" && <button type="button" className="small-secondary" disabled={blocked} onClick={event => void select("vad", event.currentTarget)}>VADモデルを選択（任意）</button>}{files.some(file => file.role === "vad") && <p>{modelFileName(files.find(file => file.role === "vad")!.path)}</p>}{files.some(file => ["runtime_library", "vad"].includes(file.role)) && <button type="button" className="small-secondary" disabled={blocked} onClick={() => { setFiles(previous => previous.filter(file => !["runtime_library", "vad"].includes(file.role))); onChanged(); }}>追加ファイルの選択を解除</button>}</details>
    <p className="field-note">選択・検査では実行や設定変更を行いません。</p>
    {error && <p role="alert">{error}</p>}
    <button type="button" disabled={blocked || !ready} onClick={() => { try { const source = guidedModelSource(engine, files); setError(""); void onInspect(source); } catch (cause) { setError(String(cause)); } }}>選択したファイルを検査</button>
  </section>;
}
