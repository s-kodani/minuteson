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
  files: ModelFileReport[];
  totalBytes: number;
  engineCompatibility: "not_verified";
  publisherAuthenticity: "not_verified";
  installed: false;
  ready: false;
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
