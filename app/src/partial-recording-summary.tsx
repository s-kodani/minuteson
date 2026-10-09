import {
  canRecoverPartialRecording, partialLaneLabel, partialLaneStateLabel,
  partialTimelineLabel, type PartialRecordingSummary as Summary,
} from "./partial-recording-state";

// Presentation only. The confirmation modal/controller must retain its native
// capability and resolve an uncertain command result before permitting retry.
export function PartialRecordingSummary({ summary }: { summary: Summary }) {
  return <section aria-label="録音の部分救出範囲">
    <h3>確定済み音声の部分救出</h3>
    <p>元の録音を正常終了として扱わず、検証できた確定範囲だけを救出します。録音は再開しません。</p>
    <p>実際の取得終了・UTC原点・末尾の損失時間は不明です。末尾損失5秒以内は保証しません。</p>
    <p>未確定・破損・検証できない音声と、元の記録は保持します。</p>
    {canRecoverPartialRecording(summary)
      ? <p>救出する時間軸の範囲: 0〜{partialTimelineLabel(summary.durationUs)}。この範囲にも欠損が含まれる場合があります。</p>
      : <p role="status">現在、救出できる確定済み音声はありません。</p>}
    <ul>{summary.lanes.map(lane => <li key={lane.lane}>
      <h4>{partialLaneLabel(lane.lane)}</h4>
      <p>{partialLaneStateLabel(lane.state)}</p>
      {lane.state === "available" && <p>
        {lane.sealedSegments}セグメント・{lane.frames}フレーム、
        {partialTimelineLabel(lane.startUs!)}〜{partialTimelineLabel(lane.endUs!)}
        （{lane.sampleRate}Hz・{lane.channels}ch）。区間内の欠損はそのまま保持します。
      </p>}
    </li>)}</ul>
  </section>;
}
