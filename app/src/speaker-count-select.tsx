import { useId, type ReactNode } from "react";
import { InfoPopover } from "./info-popover";

export function SpeakerCountSelect({label,value,disabled,onChange,help}:{
  label:string;value:number|null;disabled?:boolean;onChange:(value:number|null)=>void;help?:ReactNode;
}) {
  const id = useId();
  return <div className="info-field"><div className="heading-with-info"><label htmlFor={id}>{label}</label>{help && <InfoPopover label={label}>{help}</InfoPopover>}</div><select id={id} value={value??""} disabled={disabled} onChange={event=>onChange(event.target.value===""?null:Number(event.target.value))}>
    <option value="">自動</option>
    {Array.from({length:100},(_,i)=><option key={i+1} value={i+1}>最大{i+1}人</option>)}
  </select></div>;
}
