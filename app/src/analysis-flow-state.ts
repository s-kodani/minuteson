import type { AnalysisJob } from "./analysis-state";
export interface AnalysisFlow {
  readyToPublish?: boolean;
  replacementRequested?: boolean;
  id:string;meetingId:string;stage:string;lockVersion:number;
  profile?:"standard"|"high_accuracy";
  transcriptionJob:AnalysisJob|null;diarizationJob:AnalysisJob|null;errorMessage:string|null;
}
