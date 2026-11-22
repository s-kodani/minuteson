import { useAppName } from "./window-environment";
export function StartupLoading() {
  const appName = useAppName();
  return <main className="startup-loading" aria-label="起動確認">
    <div className="startup-loading-group">
      <p role="status">{appName}を起動しています…</p>
      <div className="startup-loading-progress" role="progressbar" aria-label="起動処理中"><span aria-hidden="true" /></div>
    </div>
  </main>;
}
