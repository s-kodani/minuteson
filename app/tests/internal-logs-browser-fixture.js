// Real application shell with synthetic native log pages; no user data or engines.
await import("./browser-fixture.js");
const baseInvoke = window.__TAURI_INTERNALS__.invoke;
const fixture = window.internalLogsFixture = {
  calls: [], entries: [], fail: false, failOlder: false, stallOlder: false, hold: false, holdOlder: false, pending: [], pages: [],
  status: { storageError: null, droppedEntries: 0, historyTruncated: false },
  add(patch = {}) {
    const id = (this.entries.at(-1)?.id ?? 0) + 1;
    const entry = { id, timestamp: 1792150000000 + id * 1000, sessionId: "synthetic-session", kind: "process", level: "info", category: "transcription", meetingId: "meeting-0", jobId: "job-synthetic", operationId: null, state: "running", elapsedMs: id * 1000, message: `ログの工程 ${id}`, stream: null, ...patch };
    this.entries.push(entry); return entry;
  },
  release() { this.hold = false; this.holdOlder = false; this.pending.splice(0).forEach(resolve => resolve()); },
};
for (let i = 0; i < 1100; i++) fixture.add({ level: i % 10 === 0 ? "warn" : "info" });
fixture.add({ kind: "engine", level: "raw", stream: "stderr.log", source: "/synthetic/unit-0", elapsedMs: null, message: "llama_model_load: using device MTL0\n<script>window.logMarkupExecuted = true</script>\n会議本文🦀\n" });
window.__TAURI_INTERNALS__.invoke = async (command, args, options) => {
  if (command !== "get_internal_logs") return baseInvoke(command, args, options);
  fixture.calls.push(structuredClone(args));
  if (fixture.hold || fixture.holdOlder && args.query.before !== undefined) await new Promise(resolve => fixture.pending.push(resolve));
  if (fixture.fail) throw Error("合成ログ取得エラー");
  const { filter = {}, before, after, limit = 200 } = args.query;
  if (before !== undefined && fixture.failOlder) throw Error("合成の履歴取得エラー");
  if (before !== undefined && fixture.stallOlder) return { entries: [], olderCursor: before, newerCursor: 1101, hasMore: true, expired: false, status: structuredClone(fixture.status) };
  if (fixture.pages.length) return structuredClone(fixture.pages.shift());
  let matches = fixture.entries.filter(entry => (!filter.kind || entry.kind === filter.kind) && (!filter.level || entry.kind === "engine" || entry.level === filter.level) && (!filter.category || entry.category === filter.category) && (!filter.text || entry.message.toLowerCase().includes(filter.text.toLowerCase())) && (!filter.relatedId || [entry.meetingId, entry.jobId, entry.operationId].some(id => id?.includes(filter.relatedId))) && (filter.from === undefined || entry.timestamp >= filter.from) && (filter.to === undefined || entry.timestamp <= filter.to) && (before === undefined || entry.id < before) && (after === undefined || entry.id > after));
  const hasMore = matches.length > limit;
  matches = after === undefined ? matches.slice(-limit) : matches.slice(0, limit);
  return { entries: structuredClone(matches), olderCursor: matches[0]?.id ?? before ?? 0, newerCursor: after !== undefined && hasMore ? matches.at(-1).id : fixture.entries.at(-1)?.id ?? 0, hasMore, expired: false, status: structuredClone(fixture.status) };
};
