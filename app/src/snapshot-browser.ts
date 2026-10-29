export const browserSchedule = (read: () => void, intervalMs: number) => {
  const timer = window.setInterval(read, intervalMs);
  return () => window.clearInterval(timer);
};
export const browserFocus = (read: () => void) => {
  window.addEventListener("focus", read);
  return () => window.removeEventListener("focus", read);
};
