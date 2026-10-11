import { useEffect, useState } from "react";
import type { DocumentPage } from "./minutes-pagination";
import { IconButton } from "./action-icon";

export function MinutesPages({ plan, mode, disabled = false, changePage }: {
  plan: DocumentPage; mode: "閲覧" | "編集"; disabled?: boolean; changePage: (page: number) => void;
}) {
  const [number, setNumber] = useState(String(plan.page + 1));
  const [error, setError] = useState("");
  useEffect(() => { setNumber(String(plan.page + 1)); setError(""); }, [plan.page]);
  if (plan.pages <= 1) return null;
  return <nav className="minutes-page-navigation" aria-label={`議事録の${mode}ページ`}>
    <div className="transcript-pages">
      <IconButton label={`前の${mode}ページ`} icon="previous" disabled={disabled || plan.page === 0} onClick={() => changePage(plan.page - 1)} />
      <span>{plan.page + 1} / {plan.pages}（本文・表の行・リスト項目を計50件ずつ表示）</span>
      <IconButton label={`次の${mode}ページ`} icon="next" disabled={disabled || plan.page + 1 >= plan.pages} onClick={() => changePage(plan.page + 1)} />
    </div>
    <form className="transcript-pages" onSubmit={(event) => {
      event.preventDefault(); if (disabled) return;
      const value = /^\d+$/.test(number) ? Number(number) : NaN;
      if (!Number.isSafeInteger(value) || value < 1 || value > plan.pages) { setError(`1〜${plan.pages}のページ番号を入力してください`); return; }
      setError(""); changePage(value - 1);
    }}>
      <label>{mode}ページ番号 <input type="number" aria-label={`議事録の${mode}ページ番号`} min={1} max={plan.pages} step={1} value={number} disabled={disabled} onChange={(event) => setNumber(event.target.value)} /></label>
      <button type="submit" disabled={disabled}>{mode}ページへ移動</button>
    </form>
    {error && <p className="action-error" role="alert">{error}</p>}
  </nav>;
}
