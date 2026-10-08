import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useState,useSyncExternalStore} from "react";
import {CaptureRecoveryClient} from "./capture-test-recovery-client";

export function useCaptureTestRecovery(){
  const [client]=useState(()=>new CaptureRecoveryClient({read:()=>invoke("get_capture_test_recovery"),
    retry:expectedRevision=>invoke("retry_capture_test_recovery",{expectedRevision})}));
  const view=useSyncExternalStore(client.subscribe,client.getSnapshot);
  useEffect(()=>{if(!isTauri())return;void client.poll().catch(()=>{});
    const timer=window.setInterval(()=>{void client.poll().catch(()=>{});},500);return()=>window.clearInterval(timer);},[client]);
  return {client,...view,blocked:client.blocked()};
}
export function CaptureTestRecoveryPanel({control,locked,currentTest}:{control:ReturnType<typeof useCaptureTestRecovery>;locked:boolean;currentTest:boolean}){
  const status=control.snapshot;
  if(control.connected&&status?.phase==="clear")return null;
  return <section className="status-card" aria-label="テスト音声の回復">
    <h2>テスト音声の回復</h2>
    <p role="status">{!control.connected?"以前のテスト音声の回復状態を確認できていません。":status?.phase==="checking"?"以前のテスト音声の所有と取得プロセスの終了を確認しています。":"以前のテスト音声を保持しています。"}</p>
    <p>確認が済むまで録音は開始できません。所有が不明なファイルは削除しません。状態の読取りでは回収を再実行しません。</p>
    {status?.phase==="attention"&&<><p>再検査では取得の終了と保存先の所有を確かめ、同じテストの既知の一時音声だけを回収します。会議の保存音声は変更しません。未回収のまま終了する場合は音声を保持します。</p>
      {currentTest&&<p>現在のテストを停止して一時音声を閉じてから、回復を再検査してください。</p>}
      <button type="button" disabled={locked||currentTest||control.commandBusy||!control.connected} onClick={()=>void control.client.retry(status.revision).catch(()=>{})}>終了と所有を再検査して回収</button>
    </>}
    {status?.errorMessage&&<p role="alert" className="action-error">{status.errorMessage}</p>}
    {control.error&&<p role="alert" className="action-error">{control.error}</p>}
    <button type="button" className="small-secondary" disabled={locked||control.commandBusy} onClick={()=>void control.client.refresh().catch(()=>{})}>回復の状態を再確認</button>
  </section>;
}
