import type {VisibleLevel} from "./input-level-state";

export const waveformBars=64;
// Each bar is one accepted native peak measurement, never a decorative signal.
// Immutable revisions cannot fill the history by being polled repeatedly.
export class RecorderWaveformHistory {
  private revision:number|null=null;
  private values:readonly number[]=[];
  read(revision:number|null,level:VisibleLevel,connected:boolean):readonly number[]{
    if(!connected||level.state!=="measured"||revision===null){
      this.revision=revision;
      if(this.values.length)this.values=[];
      return this.values;
    }
    if(this.revision!==null&&revision<=this.revision)return this.values;
    this.revision=revision;
    this.values=[...this.values.slice(-(waveformBars-1)),Math.min(1,level.peak)];
    return this.values;
  }
}
