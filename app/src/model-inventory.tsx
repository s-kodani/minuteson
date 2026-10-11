import type { ModelPlacementEntry } from "./model-inspection-state";
import type { ModelProfileView } from "./model-profile-state";
import { installedModels, modelUsage } from "./model-inventory-state";

export function ModelInventory({ entries, profiles, loading, error }: { entries: ModelPlacementEntry[]; profiles: ModelProfileView[] | null; loading: boolean; error: string }) {
  const loaded = installedModels(entries);
  return <section className="model-inventory" aria-label="導入済みモデル">
    <h3>導入済みモデル</h3>
    {loading && <p role="status">モデルを読み込んでいます。</p>}
    {error && <p role="alert">モデル一覧を確認できません: {error}</p>}
    {!loading && !error && loaded.length === 0 && <p className="empty-state">まだモデルがありません。</p>}
    {!!loaded.length && <ul>{loaded.map(entry => <li key={entry.packageId}><div className="model-inventory-name"><strong>{entry.modelName}</strong><span>{({ "whisper.cpp": "文字起こし", "sherpa-onnx": "話者分離", "llama.cpp": "議事録生成" } as Record<string, string>)[entry.engine] ?? entry.engine} · {entry.modelVersion} · {(entry.totalBytes / 1024 ** 2).toFixed(1)} MiB</span></div><span className="model-inventory-state">{error ? "利用状態を確認できません" : modelUsage(entry, profiles)}</span></li>)}</ul>}
    {!loading && !error && entries.some(entry => !entry.placementRecorded) && <p role="status">未完了の取り込みがあります。途中のファイルは保持しています。</p>}
  </section>;
}
