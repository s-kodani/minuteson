import type { AnalysisJob } from "./analysis-state";
export interface AnalysisFlow {
  readyToPublish?: boolean;
  replacementRequested?: boolean;
  id:string;meetingId:string;stage:string;lockVersion:number;
  profile?:"standard"|"high_accuracy";
  transcriptionJob:AnalysisJob|null;diarizationJob:AnalysisJob|null;errorMessage:string|null;
}
export function flowStages(flow:AnalysisFlow) {
  const label=(job:AnalysisJob|null,waiting:string)=>job ? (
    job.pauseReason==="recording"&&["pausing","paused"].includes(job.state)
      ? job.state==="pausing"?"録音・取り込み優先で停止処理中":"録音・取り込み優先で一時停止"
      : ({queued:"待機中",running:"実行中",pausing:"停止処理中",paused:"一時停止",interrupted:"中断・再開確認が必要",succeeded:"完了",failed:"失敗・同条件で再試行できます",cancelled:"取消済み"} as Record<string,string>)[job.state]??job.state
  ):waiting;
  return [
    {name:"音声の準備",state:"保存済み"},
    {name:"文字起こし",state:label(flow.transcriptionJob,flow.stage==="configuration"?"AI準備待ち":"待機中")},
    {name:"話者分離",state:label(flow.diarizationJob,"待機中")},
    {name:"結果の整理",state:flow.stage==="review"?"候補を保存済み":"待機中"},
  ];
}
export function flowContinueLabel(flow:AnalysisFlow):string|null {
  return flow.stage==="configuration"?"AI設定を確認して解析を開始":flow.stage==="attention"?"固定した条件で引継ぎを再試行":null;
}
