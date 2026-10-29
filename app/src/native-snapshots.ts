import { listen } from "@tauri-apps/api/event";
/** One cleanup covers every native invalidation channel, including a partial
 * subscribe failure and unsubscribe before the last listener is established. */
export async function subscribeInvalidations(events: readonly string[], notify: () => void): Promise<() => void> {
  const results = await Promise.allSettled(events.map(event => listen(event, notify)));
  const stops = results.flatMap(result => result.status === "fulfilled" ? [result.value] : []);
  const failure = results.find(result => result.status === "rejected");
  if (failure?.status === "rejected") { stops.forEach(stop => stop()); throw failure.reason; }
  return () => stops.forEach(stop => stop());
}
