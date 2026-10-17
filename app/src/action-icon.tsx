import type { ComponentPropsWithoutRef, Ref } from "react";

export type ActionIconName = "voice" | "filter" | "logs" | "gear" | "trash" | "bell" | "close" | "open" | "edit" | "preview" | "play" | "pause" | "stop" | "restart" | "more" | "refresh" | "undo" | "plus" | "previous" | "next" | "settings" | "download" | "bold" | "italic" | "heading1" | "heading2" | "heading3" | "list" | "numberedList" | "table";

// One code-native icon style; decorative SVGs never become the button's name.
export function ActionIcon({ name }: { name: ActionIconName }) {
  return <svg className="action-icon" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
    {name === "voice" && <><path d="M3 10v4M7 6v12M11 3v18M15 7v10M19 5v14M23 10v4" /></>}
    {name === "filter" && <path d="M3 4h18l-7 8v7l-4 2v-9L3 4Z" />}
    {name === "logs" && <><rect x="4" y="3" width="16" height="18" rx="2" /><path d="M8 8h8M8 12h8M8 16h5" /></>}
    {name === "bell" && <><path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4" /></>}
    {name === "gear" && <><path d="m9 3-.6 2.2-1.9 1.1-2.2-.6-2 3.5L4 10.8v2.4l-1.7 1.6 2 3.5 2.2-.6 1.9 1.1L9 21h4l.6-2.2 1.9-1.1 2.2.6 2-3.5-1.7-1.6v-2.4l1.7-1.6-2-3.5-2.2.6-1.9-1.1L13 3Z" /><circle cx="11" cy="12" r="3" /></>}
    {name === "trash" && <><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7" /></>}
    {name === "close" && <path d="m6 6 12 12M18 6 6 18" />}
    {name === "open" && <><path d="M14 3h7v7M21 3 11 13M10 4H4v16h16v-6" /></>}
    {name === "edit" && <><path d="m14 5 5 5M4 20l4.5-1L20 7.5a2.8 2.8 0 0 0-4-4L4.5 15 4 20Z" /><path d="M13 20h7" /></>}
    {name === "preview" && <><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z" /><circle cx="12" cy="12" r="3" /></>}
    {name === "play" && <path d="m8 5 11 7-11 7V5Z" />}
    {name === "pause" && <><path d="M7 5h3v14H7zM14 5h3v14h-3z" /></>}
    {name === "stop" && <rect x="4" y="4" width="16" height="16" rx="2" fill="currentColor" stroke="none" />}
    {name === "restart" && <><path d="M5 5v14m14-14L8 12l11 7V5Z" /></>}
    {name === "more" && <><circle cx="5" cy="12" r="1" fill="currentColor" /><circle cx="12" cy="12" r="1" fill="currentColor" /><circle cx="19" cy="12" r="1" fill="currentColor" /></>}
    {name === "refresh" && <><path d="M20 7a8 8 0 0 0-14-1L3 9m0-5v5h5M4 17a8 8 0 0 0 14 1l3-3m0 5v-5h-5" /></>}
    {name === "undo" && <><path d="M8 5 3 10l5 5M3 10h10a7 7 0 0 1 7 7v2" /></>}
    {name === "plus" && <path d="M12 5v14M5 12h14" />}
    {name === "previous" && <path d="m15 5-7 7 7 7" />}
    {name === "next" && <path d="m9 5 7 7-7 7" />}
    {name === "settings" && <><path d="M4 7h16M4 17h16" /><circle cx="9" cy="7" r="3" fill="var(--surface)" /><circle cx="15" cy="17" r="3" fill="var(--surface)" /></>}
    {name === "download" && <><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5" /></>}
    {name === "bold" && <path d="M6 12h7a4 4 0 0 1 0 8H6V4h6a4 4 0 0 1 0 8" strokeWidth="2.4" />}
    {name === "italic" && <path d="M10 4h9M5 20h9M15 4 9 20" />}
    {(["heading1", "heading2", "heading3"] as ActionIconName[]).includes(name) && <path d="M3 5v14M12 5v14M3 12h9" />}
    {name === "heading1" && <path d="m17 12 3-2v10M17 20h6" />}
    {name === "heading2" && <path d="M16 12a3 3 0 0 1 6 0c0 2-6 4-6 8h6" />}
    {name === "heading3" && <path d="M16 10h3a2.5 2.5 0 0 1 0 5h-2m2 0a2.5 2.5 0 0 1 0 5h-3" />}
    {name === "list" && <><path d="M9 6h12M9 12h12M9 18h12" /><circle cx="4" cy="6" r="1" fill="currentColor" /><circle cx="4" cy="12" r="1" fill="currentColor" /><circle cx="4" cy="18" r="1" fill="currentColor" /></>}
    {name === "numberedList" && <path d="M10 6h11M10 12h11M10 18h11M3 3h2v6M3 9h4M3 14a2 2 0 0 1 4 0c0 1-4 3-4 5h4" />}
    {name === "table" && <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 10h18M3 15h18M9 10v10M15 10v10" /></>}
  </svg>;
}

export function IconButton({ label, icon, className = "", ref, ...props }: Omit<ComponentPropsWithoutRef<"button">, "children" | "aria-label" | "title"> & { label: string; icon: ActionIconName; ref?: Ref<HTMLButtonElement> }) {
  return <button {...props} ref={ref} type={props.type ?? "button"} className={`small-secondary icon-button ${className}`} aria-label={label} title={label}><ActionIcon name={icon} /></button>;
}
