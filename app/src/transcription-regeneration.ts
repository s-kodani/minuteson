import type { AnalysisFlow } from "./analysis-flow-state";
import type { ConfirmationOptions, ConfirmationOutcome } from "./confirmation-state";
import { withConfirmedEdits } from "./confirmed-action.ts";

export type ReplacementTarget = NonNullable<RegenerationTarget["target"]>;
export type Publication = { status: "published"; flow: AnalysisFlow } | {
  status: "confirmationRequired"; flow: AnalysisFlow; target: ReplacementTarget; meetingTitle: string;
};

export interface RegenerationTarget {
  target: { analysisId: string; contentHash: string; draft: { id: string; contentHash: string; lockVersion: number } | null } | null;
  flow: { id: string; lockVersion: number } | null;
}
export interface RegenerationRequest {
  meetingId: string;
  operationId: string;
  expected: RegenerationTarget;
  numSpeakers?: number | null;
}
// Release the edit barrier before showing the modal, then save and compare the
// confirmed draft again on acceptance. No model execution is needed here.
export async function publishWithReplacementConfirmation(flow: AnalysisFlow, deps: {
  saveThenAct: (action: () => Promise<void>) => Promise<void | "cancelled">;
  publish: (flow: AnalysisFlow, target?: ReplacementTarget) => Promise<Publication>;
  confirm: (options: ConfirmationOptions) => Promise<ConfirmationOutcome>;
}) {
  let receipt: Publication | undefined;
  await withConfirmedEdits(deps.saveThenAct, async () => { receipt = await deps.publish(flow); });
  if (receipt?.status !== "confirmationRequired") return "completed";
  const pending = receipt;
  return deps.confirm({
    title: "生成結果に置き換えますか？", target: `会議「${pending.meetingTitle}」`,
    message: "再生成の開始後に文字起こしが編集されています。置き換えると、現在の本文・話者の修正を生成済みの結果で上書きします。議事録は変更しません。キャンセルした場合は現在の修正と生成結果を保持し、あとから置き換えを確認できます。",
    confirmLabel: "置き換える",
    action: () => withConfirmedEdits(deps.saveThenAct, async () => {
      const result = await deps.publish(pending.flow, pending.target);
      if (result.status !== "published") throw Error("文字起こしが変更されています。置き換えの確認を開き直してください。");
    }),
  });
}
