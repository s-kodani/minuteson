import test from "node:test";
import assert from "node:assert/strict";
import { backupNotification } from "../src/backup-notification.ts";
import type { DataSnapshot } from "../src/data-operation-state.ts";
const snapshot = (destination:string) => ({kind:"backup",phase:"completed",busy:false,backupPreview:{destination},backupReceipt:{meetingCount:2,size:1024,deletedAudioMeetingCount:0,deletedAudioAssetCount:0}} as DataSnapshot);
test("backup notifications bind to each completed operation and display Windows paths",()=>{
  const b=String.fromCharCode(92),path=b.repeat(2)+"?"+b+["C:","会議 データ","会議.monbackup"].join(b),a=snapshot(path),other=snapshot("/safe/別.monbackup");
  const notice=backupNotification(a)!;
  assert.ok(notice.details.includes("保存先: "+path.slice(4)));
  assert.ok(!notice.details.includes(b.repeat(2)+"?"));
  assert.ok(backupNotification(other)!.details.includes("/safe/別.monbackup"));
  assert.equal(a.backupPreview?.destination,path);
});
test("partial and uncertain backup outcomes never receive a success notice",()=>{
  for(const phase of ["publishing","cancelled","failed","attention"])assert.equal(backupNotification({...snapshot("/safe/a.monbackup"),phase}),null);
  assert.equal(backupNotification({...snapshot("/safe/a.monbackup"),backupReceipt:null}),null);
  assert.match(backupNotification({...snapshot("/safe/a.monbackup"),backupReceipt:{...snapshot("").backupReceipt!,deletedAudioMeetingCount:1,deletedAudioAssetCount:4}})!.details,/音声本体は含みません/);
});
