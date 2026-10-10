import { useEffect, useId, useState, type ReactNode } from "react";

// Presentation only. Keep inputs/controllers mounted, and retain an automatic
// opening after attention clears so a focused control is not suddenly hidden.
export function SettingsDisclosure({ label, attention = false, initialOpen = false, children }: {
  label: string; attention?: boolean; initialOpen?: boolean; children: ReactNode;
}) {
  const [open, setOpen] = useState(initialOpen);
  const id = useId();
  const expanded = open || attention;
  useEffect(() => { if (attention) setOpen(true); }, [attention]);
  return <div className="settings-disclosure">
    <button type="button" className="small-secondary settings-disclosure-toggle" aria-expanded={expanded} aria-controls={id} disabled={attention} onClick={() => setOpen(!expanded)}>{label}<span aria-hidden="true">{expanded ? " ▴" : " ▾"}</span></button>
    <div id={id} className="settings-disclosure-body" hidden={!expanded}>{children}</div>
  </div>;
}
