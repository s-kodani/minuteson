export interface ModelFileReport {
  relativePath: string;
  role: string;
  actualSize: number;
  actualHash: string;
  expectedSizeChecked: boolean;
  expectedHashChecked: boolean;
}
export interface ModelInspectionReport {
  manifest: { modelName: string; modelVersion: string; engine: string; engineVersion: string; target: string; licenseRef: string; quantization: string | null };
  manifestHash: string;
  reviewHash: string;
  files: ModelFileReport[];
  totalBytes: number;
  engineCompatibility: "not_verified";
  publisherAuthenticity: "not_verified";
  installed: false;
  ready: false;
}

export interface ModelPlacementReceipt {
  packageId: string;
  reviewHash: string;
  totalBytes: number;
  placementRecorded: boolean;
  engineCompatibility: "not_verified";
  ready: false;
}
export interface ModelPlacementEntry {
  packageId: string;
  modelName: string;
  modelVersion: string;
  engine: string;
  engineVersion: string;
  totalBytes: number;
  placementRecorded: boolean;
  ready: false;
  reviewHash: string;
  compatibility?: ModelCompatibilitySummary | null;
}
export interface ModelCompatibilitySummary {
  operationId: string;
  packageId: string;
  reviewHash: string;
  adapterRevision: number;
  state: "preparing" | "succeeded" | "failed" | "cancelled" | "interrupted";
  errorMessage: string | null;
}
const canonicalId = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
function validateCompatibility(summary: ModelCompatibilitySummary) {
  if (!summary || !canonicalId.test(summary.operationId) || !canonicalId.test(summary.packageId) || !/^[0-9a-f]{64}$/.test(summary.reviewHash) || summary.adapterRevision !== 1 || !["preparing", "succeeded", "failed", "cancelled", "interrupted"].includes(summary.state) ||
    (["preparing", "succeeded"].includes(summary.state) ? summary.errorMessage !== null : typeof summary.errorMessage !== "string" || !summary.errorMessage || new TextEncoder().encode(summary.errorMessage).length > 4096 || summary.errorMessage.includes("\0"))) {
    throw Error("互換性確認の記録が不正です。配置記録を再読込してください。");
  }
}
export function assertCompatibilitySummary(summary: ModelCompatibilitySummary, token: string, entry: Pick<ModelPlacementEntry, "packageId" | "reviewHash">) {
  validateCompatibility(summary);
  if (summary.operationId !== token || summary.packageId !== entry.packageId || summary.reviewHash !== entry.reviewHash || !["succeeded", "failed", "cancelled"].includes(summary.state)) {
    throw Error("互換性の応答と確認した素材・操作が一致しません。配置記録を再読込してください。");
  }
}
export function placementLabel(entry: Pick<ModelPlacementEntry, "placementRecorded" | "ready"> & Partial<Pick<ModelPlacementEntry, "compatibility" | "packageId" | "reviewHash">>): string {
  if (entry.ready !== false) throw Error("配置記録だけでAI準備完了とは扱えません。");
  if (entry.compatibility) {
    validateCompatibility(entry.compatibility);
    if (!entry.placementRecorded || entry.compatibility.packageId !== entry.packageId || entry.compatibility.reviewHash !== entry.reviewHash) throw Error("配置と互換性の記録が一致しません。");
    switch (entry.compatibility.state) {
      case "preparing": return "互換性確認中";
      case "succeeded": return "CPU互換性確認済み（適用状態は構成記録を参照）";
      case "failed": return "互換性確認失敗";
      case "cancelled": return "互換性確認取消・明示的な再確認が必要";
      case "interrupted": return "前回の互換性確認が中断・明示的な再確認が必要";
    }
  }
  return entry.placementRecorded ? "配置記録あり・互換性検証待ち" : "未完了の配置・途中ファイルは保全";
}
export function assertPlacementReceipt(receipt: ModelPlacementReceipt, token: string, reviewHash: string) {
  if (receipt.packageId !== token || receipt.reviewHash !== reviewHash || receipt.ready !== false || receipt.placementRecorded !== true || receipt.engineCompatibility !== "not_verified") {
    throw Error("配置の応答と確認した素材が一致しません。配置記録を再読込してください。");
  }
}

// A returned preflight must never be presented as installation or readiness.
export function inspectionLabel(report: ModelInspectionReport): string {
  if (report.installed !== false || report.ready !== false || report.engineCompatibility !== "not_verified" || report.publisherAuthenticity !== "not_verified") {
    throw Error("事前検査とモデル準備の状態が一致しません。再検査してください。");
  }
  return "ファイル検査済み・未導入（エンジン互換性は未検証）";
}

export class InspectionAttempt {
  private current: { token: string; cancelled: boolean } | null = null;
  begin(token: string) { this.current = { token, cancelled: false }; }
  cancel(): string | null {
    if (!this.current) return null;
    this.current.cancelled = true;
    return this.current.token;
  }
  accepts(token: string): boolean { return this.current?.token === token && !this.current.cancelled; }
  finish(token: string): boolean {
    if (this.current?.token !== token) return false;
    this.current = null;
    return true;
  }
}
