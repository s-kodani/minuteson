import { MarkdownSaveController, type MarkdownDocument } from "./minutes-markdown-state.ts";
import type { ConfirmationOptions, ConfirmationOutcome } from "./confirmation-state";
import { SavedEditsCancelled } from "./confirmed-action.ts";

// Navigation, native quit and explicit regeneration share the same decision.
// A save is never implicit, and unknown save responses cannot be discarded by
// merely reading the old base while the original native write may still run.
export async function confirmMarkdownLeave(controller: MarkdownSaveController, title: string,
  read: () => Promise<MarkdownDocument | null>,
  confirm: (options: ConfirmationOptions) => Promise<ConfirmationOutcome>) {
  if (controller.isComposing()) throw Error("日本語入力を確定してから操作してください。入力は保持しています。");
  if (controller.snapshot().saving) await controller.save();
  const state = controller.snapshot();
  if (!state.dirty) return;
  const unchanged = () => {
    const current = controller.snapshot();
    return !current.saving && !controller.isComposing() && current.text === state.text
      && current.document?.operationId === state.document?.operationId;
  };
  const outcome = await confirm({ title: "議事録の未保存の編集", target: `会議「${title}」`,
    message: "議事録に未保存の編集があります。保存して続けるか、編集を破棄して保存済みの本文へ戻してください。",
    confirmLabel: "保存して続ける", validate: unchanged,
    action: async () => {
      await controller.save();
      if (controller.snapshot().dirty) throw Error("保存後も未保存の入力があります。入力を確認し、改めて保存してください。");
    }, alternativeAction: { label: "編集を破棄して続ける", action: async () => {
      const document = await read();
      if (!unchanged()) throw Error("確認中に入力や保存状態が変わりました。入力は保持しています。");
      controller.discard(document);
    } },
  });
  if (outcome === "cancelled") throw new SavedEditsCancelled();
  if (outcome !== "completed") throw Error("議事録の保存結果を確認できません。編集を保持しています。");
}
