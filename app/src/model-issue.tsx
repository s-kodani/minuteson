// Onboarding errors stay readable in place, without another help popup.
export function ModelIssue({ label, message }: { label: string; message: string | null | undefined }) {
  return message ? <p className="action-error" role="alert" aria-label={`${label}の要対応状態`}>{label}: {message}</p> : null;
}
