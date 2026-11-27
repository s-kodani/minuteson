import type {VisibleLevel} from "./input-level-state";

export const waveformBars=64;
export type WaveformSample=number|null;
const freshMs=1000,defaultSlotMs=250,minSlotMs=100,maxSlotMs=500,intervalSamples=16;
// Each bar is one accepted native peak measurement, never a decorative signal.
// Without a fresh measurement the time axis keeps moving: blank slots enter on
// the right and old bars scroll out, so the right edge never shows old audio as
// current and a gap's width reflects how long the input was missing.
export class RecorderWaveformHistory {
  private revision:number|null=null;
  private values:readonly WaveformSample[]=[];
  private clock:{mono:number;wall:number}|null=null;
  // A private axis: monotonic and wall advances both count, reversals never
  // rewind it, and a rebuilt level clock does not reset it.
  private axis=0;
  // Axis time represented by the right-most slot.
  private cursor=0;
  private lastBar:number|null=null;
  private intervals:number[]=[];
  // One slot spans the mean spacing of recent consecutive measurements, so gaps
  // and bars share a scale on 100 ms (macOS) and 250 ms (Windows) publishers.
  // A mean, not a median: 100 ms paints observe a 250 ms publisher as
  // alternating 200 ms and 300 ms intervals.
  private slotMs=defaultSlotMs;
  read(revision:number|null,level:VisibleLevel,connected:boolean,monoMs:number,wallMs:number):readonly WaveformSample[]{
    this.advance(monoMs,wallMs);
    let next=this.values;
    const newer=revision!==null&&(this.revision===null||revision>this.revision);
    if(newer)this.revision=revision;
    const fresh=connected&&level.state==="measured"&&revision!==null;
    if(fresh&&newer){
      const right=next.at(-1);
      if(right===null)next=this.fill(next);
      else if(right!==undefined&&this.lastBar!==null&&this.axis-this.lastBar>freshMs)next=this.blanks(next,Math.floor((this.axis-this.lastBar)/this.slotMs)-1);
      else if(right!==undefined&&this.lastBar!==null)this.observe(this.axis-this.lastBar);
      next=[...next.slice(-(waveformBars-1)),Math.min(1,level.peak)];
      this.lastBar=this.axis;this.cursor=this.axis;
    }else{
      if(!fresh&&typeof next.at(-1)==="number"){
        // The right edge turns blank at once, covering any time since the last bar.
        const slots=Math.floor((this.axis-this.cursor)/this.slotMs);
        next=this.blanks(next,Math.max(1,slots));
        this.cursor=slots?this.cursor+slots*this.slotMs:this.axis;
      }else if(next.at(-1)===null||!next.length)next=this.fill(next);
    }
    if(next.length&&next.every(value=>value===null))next=[];
    if(next.length!==this.values.length||next.some((value,i)=>value!==this.values[i]))this.values=next;
    return this.values;
  }
  private advance(mono:number,wall:number){
    if(!Number.isFinite(mono)||!Number.isFinite(wall))return;
    if(this.clock)this.axis+=Math.max(0,mono-this.clock.mono,wall-this.clock.wall);
    else this.cursor=this.axis;
    this.clock={mono,wall};
  }
  private observe(interval:number){
    this.intervals=[...this.intervals.slice(-(intervalSamples-1)),interval];
    this.slotMs=Math.min(maxSlotMs,Math.max(minSlotMs,this.intervals.reduce((sum,value)=>sum+value,0)/this.intervals.length));
  }
  // Blank slots for the whole time since the right-most slot.
  private fill(values:readonly WaveformSample[]){
    const count=Math.floor((this.axis-this.cursor)/this.slotMs);
    if(count<=0)return values;
    this.cursor+=count*this.slotMs;
    return this.blanks(values,count);
  }
  private blanks(values:readonly WaveformSample[],count:number){
    if(count<=0)return values;
    if(count>=waveformBars)return Array<WaveformSample>(waveformBars).fill(null);
    return [...values,...Array<WaveformSample>(count).fill(null)].slice(-waveformBars);
  }
}
