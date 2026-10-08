// Synthetic recorder memory only. Reload persistence simulates a surviving
// native recorder, never a process-restart journal or real acquisition.
const clone = value => structuredClone(value);
const valid = value => /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value);
const target = () => new URLSearchParams(location.search).get("window") === "recorder" ? "recorder" : "main";
function project(head) {
  const { claimedAt, ...notice } = head;
  return { ...clone(notice), delivery: { ...clone(notice.delivery), leaseAgeMs: claimedAt === null ? null : Date.now() - claimedAt } };
}
function receipt(ownerId, head) {
  const n = project(head);
  return { schemaVersion: 2, ownerId, noticeId: n.id, target: n.target, recipientId: n.recipientId, delivery: n.delivery };
}
export function queuedProjection(f) {
  const q = f.queueState, notices = [];
  for (const input of ["microphone", "system"]) {
    const lane = q.lanes[input];
    while (lane.length) {
      const head = lane[0];
      if (head.claimedAt !== null && Date.now() - head.claimedAt >= 5000) {
        if (head.delivery.rendered) { q.closed[head.id] = receipt(f.recording.sessionId, head); lane.shift(); continue; }
        head.delivery.generation++; head.target = null; head.recipientId = null; head.claimedAt = null; head.delivery.receiveId = null;
      }
      if (head.recipientId === null && head.target !== target()) {
        if (head.target !== null) head.delivery.generation++;
        head.target = target();
      }
      notices.push(project(head)); break;
    }
  }
  return { ...clone(f.inputStatus), notices, noticeLoss: clone(q.loss) };
}
export async function queuedAction(f, command, args) {
  const action = command.split("_")[0];
  if (f.queueHold[action]) await new Promise(resolve => f.queueReplies.push({ action, resolve }));
  if (f.failInputStatus || f.inputStatus?.reportAgeMs > 1000 || f.recording.sessionId !== args.sessionId || !["preparing", "recording"].includes(f.recording.phase) || !valid(args.recipientId)) throw Error("模擬通知所有・鮮度変更");
  const q = f.queueState, closed = q.closed[args.noticeId];
  if (action !== "receive" && closed?.recipientId === args.recipientId && closed.target === target() && closed.delivery.generation === args.generation) return clone(closed);
  const lane = Object.values(q.lanes).find(lane => lane[0]?.id === args.noticeId), head = lane?.[0];
  if (!head || head.target !== target() || head.delivery.generation !== args.generation || head.claimedAt !== null && Date.now() - head.claimedAt >= 5000) throw Error("模擬通知宛先・世代・期限変更");
  if (action === "receive") {
    if (!valid(args.receiveId)) throw Error("模擬受付操作IDなし");
    head.recipientId ??= args.recipientId;
    if (head.recipientId === args.recipientId) { head.claimedAt = Date.now(); head.delivery.receiveId = args.receiveId; }
  } else {
    if (head.recipientId !== args.recipientId || head.claimedAt === null || action === "close" && !head.delivery.rendered) throw Error("模擬表示未確認");
    head.delivery.rendered = true; head.claimedAt = Date.now();
  }
  const result = receipt(args.sessionId, head);
  if (action === "close") { q.closed[head.id] = clone(result); lane.shift(); }
  if (f.queueLost[action]) throw Error("模擬確定後の応答喪失");
  return result;
}
