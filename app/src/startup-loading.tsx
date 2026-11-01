import { useAppName } from "./window-environment";
export function StartupLoading() {
  const appName = useAppName();
  return <main className="startup-loading" aria-label="起動確認"><p role="status">{appName}を起動しています…</p></main>;
}
