export const speakerPalette = [
  ["#1d4ed8","#eff6ff","#bfdbfe","#172554"],
  ["#166534","#f0fdf4","#bbf7d0","#052e16"],
  ["#6b21a8","#faf5ff","#e9d5ff","#3b0764"],
  ["#9f1239","#fff1f2","#fecdd3","#4c0519"],
  ["#92400e","#fffbeb","#fde68a","#451a03"],
  ["#155e75","#ecfeff","#a5f3fc","#083344"],
  ["#3730a3","#eef2ff","#c7d2fe","#1e1b4b"],
  ["#115e59","#f0fdfa","#99f6e4","#042f2e"],
  ["#9a3412","#fff7ed","#fed7aa","#431407"],
  ["#86198f","#fdf4ff","#f5d0fe","#4a044e"],
  ["#334155","#f1f5f9","#cbd5e1","#0f172a"],
  ["#065f46","#ecfdf5","#a7f3d0","#022c22"],
] as const;

// Presentation only. The native UI projection supplies a persisted speaker
// lineage as colorKey, so immutable revision UUID changes cannot change hue.
export function speakerTone(meetingId:string,colorKey:string):number {
  let hash=2166136261;
  for(const value of [meetingId,"\0",colorKey])for(let index=0;index<value.length;index++)hash=Math.imul(hash^value.charCodeAt(index),16777619)>>>0;
  return hash%speakerPalette.length;
}
