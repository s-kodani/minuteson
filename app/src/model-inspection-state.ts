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
}
export function placementLabel(entry: Pick<ModelPlacementEntry, "placementRecorded" | "ready">): string {
  if (entry.ready !== false) throw Error("配置記録だけでAI準備完了とは扱えません。");
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
