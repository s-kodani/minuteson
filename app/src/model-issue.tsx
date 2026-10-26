import { ErrorNotice } from "./toast";
// Model preparation failures use the common notification history.
export function ModelIssue({ label, message }: { label: string; message: string | null | undefined }) {
  return <ErrorNotice scope={`model-issue:${label}`} type="operation" active={!!message}>{label}: {message}</ErrorNotice>;
}
