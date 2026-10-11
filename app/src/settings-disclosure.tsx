import { type ReactNode, type Ref } from "react";
import { OptionsModal } from "./options-modal";

// Attention is visible at the entry point; background polling never opens a
// modal or steals focus. Retain initialOpen as a caller compatibility option.
export function SettingsDisclosure({ label, className = "", attention = false, children, open, onOpenChange, triggerRef, restoreTarget }: {
  label: string; className?: string; attention?: boolean; initialOpen?: boolean; children: ReactNode;
  open?: boolean; onOpenChange?: (open: boolean)=>void;
  triggerRef?: Ref<HTMLButtonElement>; restoreTarget?: ()=>HTMLElement | null;
}) {
  return <OptionsModal label={label} className={`settings-disclosure ${className}`} attention={attention} open={open} onOpenChange={onOpenChange} triggerRef={triggerRef} restoreTarget={restoreTarget}>{children}</OptionsModal>;
}
