import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const source=(name:string)=>readFileSync(new URL(`../src/${name}`,import.meta.url),"utf8");
test("notifications have one Sonner presenter plus a menu-only history",()=>{
  const main=source("main.tsx"),toast=source("toast.tsx");
  assert.equal(main.match(/<NotificationList\b/g)?.length,1);
  assert.match(main,/aria-label="通知一覧"[^\n]*setNotificationsOpen\(true\)/);
  assert.match(main,/className="notifications-dialog"/);
  assert.equal(main.match(/<ToastHost\b/g)?.length,1);
  assert.doesNotMatch(main,/<RecordingInputNotice windowLabel="recorder"/);
  assert.doesNotMatch(toast,/className="issue-status"/);
  assert.match(toast,/Toaster|sonnerToast/);
  assert.match(toast,/toasts.getHistorySnapshot/);
  assert.match(toast,/return visible \?/);
  assert.match(toast,/modal&&!modal.contains\(element\)/);
});
