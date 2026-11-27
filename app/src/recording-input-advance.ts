import type {RecoverableClock,RecoverableRead} from "./recoverable-read.ts";

export function advanceSharedRecordingClock(
  clock:RecoverableClock,
  statusPoll:RecoverableRead,
  journalPoll:RecoverableRead,
  mono:()=>number,
  wall:()=>number,
):void{
  const at=clock.read(mono(),wall());
  if(Number.isNaN(at)){
    statusPoll.reverseClock();
    journalPoll.reverseClock();
    return;
  }
  statusPoll.advance();
  journalPoll.advance();
}
