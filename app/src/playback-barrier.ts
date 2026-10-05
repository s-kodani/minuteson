const stops = new Set<() => Promise<void>>();
export function registerPlaybackStop(stop: () => Promise<void>): () => void {
  stops.add(stop);
  return () => { stops.delete(stop); };
}
// Invoke every stop synchronously before awaiting any of them. Each owner
// invalidates pending preparation and awaits native token release itself.
export async function stopAllPlayback(): Promise<void> {
  await Promise.all(Array.from(stops, stop => {
    try { return stop(); } catch (cause) { return Promise.reject(cause); }
  }));
}
