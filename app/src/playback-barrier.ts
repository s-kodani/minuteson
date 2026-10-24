const stops = new Map<() => Promise<void>, string | null>();
export function registerPlaybackStop(stop: () => Promise<void>, meetingId: string | null = null): () => void {
  stops.set(stop, meetingId);
  return () => { stops.delete(stop); };
}
// Invoke every stop synchronously before awaiting any of them. Each owner
// invalidates pending preparation and awaits native token release itself.
export async function stopAllPlayback(): Promise<void> {
  await Promise.all(Array.from(stops.keys(), stop => {
    try { return stop(); } catch (cause) { return Promise.reject(cause); }
  }));
}

export async function stopMeetingPlayback(meetingId: string): Promise<void> {
  await Promise.all(Array.from(stops).filter(([, id]) => id === meetingId).map(async ([stop]) => stop()));
}
