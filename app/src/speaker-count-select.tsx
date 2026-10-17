export function SpeakerCountSelect({label,value,disabled,onChange}:{
  label:string;value:number|null;disabled?:boolean;onChange:(value:number|null)=>void;
}) {
  return <label>{label}<select value={value??""} disabled={disabled} onChange={event=>onChange(event.target.value===""?null:Number(event.target.value))}>
    <option value="">自動</option>
    {Array.from({length:100},(_,i)=><option key={i+1} value={i+1}>{i+1}人</option>)}
  </select></label>;
}
