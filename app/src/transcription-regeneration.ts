export interface RegenerationTarget {
  target: { analysisId: string; contentHash: string; draft: { id: string; contentHash: string; lockVersion: number } | null };
  flow: { id: string; lockVersion: number } | null;
}
export interface RegenerationRequest {
  meetingId: string;
  operationId: string;
  expected: RegenerationTarget;
}
