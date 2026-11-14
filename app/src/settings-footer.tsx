import { useEffect, useRef, useState } from "react";
import { getName, getVersion } from "@tauri-apps/api/app";
import { CommonModal } from "./common-modal";
import { Credits } from "./credits";
import { AppUpdateDetails, useAppUpdates } from "./app-updates";
import "./settings-footer.css";

function Disclaimer() {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  return <>
    <button ref={trigger} type="button" className="settings-footer-link" aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}>免責事項</button>
    <CommonModal portal open={open} onDismiss={() => setOpen(false)} restoreTarget={() => trigger.current} dismissLabel="閉じる" dismissText="免責事項を閉じる" aria-label="免責事項" className="settings-info-modal disclaimer-modal">
      <h2>免責事項</h2>
      <h3>AIの出力</h3>
      <p>文字起こし・話者分離・議事録には誤りや欠落が含まれる場合があります。利用・共有前に原音声や資料で確認してください。</p>
      <h3>録音と権利への配慮</h3>
      <p>録音に必要な同意や許可を確認し、プライバシー・著作権などに配慮してください。</p>
      <h3>データの保全</h3>
      <p>重要な音声・文書は定期的にバックアップしてください。</p>
      <h3>無保証・責任の制限</h3>
      <p>本アプリはMITライセンスに基づき、現状のまま無保証で提供されます。明示・黙示を問わず、商品性・特定目的への適合性・権利の非侵害を含む保証はありません。著作者および著作権者は、本アプリの使用などに起因する請求・損害・その他の責任を負いません。</p>
      <p>上記はMITライセンスの説明です。無保証・責任の制限はライセンス原文に従います。</p>
    </CommonModal>
  </>;
}

function About() {
  const updates = useAppUpdates();
  const [open, setOpen] = useState(false);
  const [metadata, setMetadata] = useState<{ name: string; version: string } | null>(null);
  const [loading, setLoading] = useState(false), [error, setError] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const alive = useRef(true), requesting = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const load = async () => {
    if (requesting.current) return;
    requesting.current = true; setLoading(true); setError("");
    try {
      const [name, version] = await Promise.all([getName(), getVersion()]);
      if (typeof name !== "string" || !name.trim() || typeof version !== "string" || !version.trim()) throw Error("アプリ情報の形式が不正です。");
      if (alive.current) setMetadata({ name, version });
    } catch (cause) { if (alive.current) setError(String(cause)); }
    finally { requesting.current = false; if (alive.current) setLoading(false); }
  };
  const show = () => { setOpen(true); if (!metadata) void load(); void updates.refresh(); };
  return <>
    <button ref={trigger} type="button" className="settings-footer-link" aria-haspopup="dialog" aria-expanded={open} onClick={show}>このアプリについて</button>
    <CommonModal portal open={open} onDismiss={() => setOpen(false)} restoreTarget={() => trigger.current} dismissLabel="閉じる" dismissText="このアプリについてを閉じる" aria-label="このアプリについて" className="settings-info-modal about-modal">
      <h2>このアプリについて</h2>
      {loading && <p role="status">アプリ情報を読み込んでいます。</p>}
      {error && <div><p role="alert">アプリ情報を読み込めませんでした。{error}</p><button type="button" className="small-secondary" disabled={loading} onClick={() => void load()}>再試行</button></div>}
      {metadata && <p className="about-app-name">{metadata.name}</p>}
      <p>会議の録音・文字起こし・話者分離・議事録生成をローカルで行うデスクトップアプリです。</p>
      <dl className="about-metadata">
        {metadata && <><dt>バージョン</dt><dd>{metadata.version}</dd></>}
        <dt>作者</dt><dd>s-kodani</dd>
        <dt>ライセンス</dt><dd>MIT License</dd>
      </dl>
      <AppUpdateDetails />
      <p className="about-copyright">Copyright (c) 2026 s-kodani</p>
    </CommonModal>
  </>;
}

export function SettingsFooter() {
  return <footer className="settings-footer" role="group" aria-label="アプリ情報">
    <Credits />
    <Disclaimer />
    <About />
  </footer>;
}
