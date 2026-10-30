const stops = new Map<() => Promise<void>, { meetingId: string | null; captureTestId: string | null }>();
export function registerPlaybackStop(stop: () => Promise<void>, meetingId: string | null = null): () => void {
  stops.set(stop, { meetingId, captureTestId: null });
  return () => { stops.delete(stop); };
}
export function registerCaptureTestPlaybackStop(stop: () => Promise<void>, runId: string): () => void {
  stops.set(stop, { meetingId: null, captureTestId: runId });
  return () => { stops.delete(stop); };
}
export async function stopCaptureTestPlayback(runId: string): Promise<void> {
  await Promise.all(Array.from(stops).filter(([, owner]) => owner.captureTestId === runId).map(async ([stop]) => stop()));
}
// Invoke every stop synchronously before awaiting any of them. Each owner
// invalidates pending preparation and awaits native token release itself.
export async function stopAllPlayback(): Promise<void> {
  await Promise.all(Array.from(stops.keys(), stop => {
    try { return stop(); } catch (cause) { return Promise.reject(cause); }
  }));
}

export async function stopMeetingPlayback(meetingId: string): Promise<void> {
  await Promise.all(Array.from(stops).filter(([, owner]) => owner.meetingId === meetingId).map(async ([stop]) => stop()));
}
