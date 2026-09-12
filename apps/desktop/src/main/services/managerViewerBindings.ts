/** Owner browser's attached viewer map, submitted with a handoff action. */
const viewers = new Map<string,string>();
export function setManagerViewerBindings(bindings: unknown): void {
  if(!Array.isArray(bindings)||bindings.length>256)throw new Error('Invalid manager viewer bindings');
  const next = new Map<string,string>();
  for(const pair of bindings){
    if(!Array.isArray(pair)||pair.length!==2||pair.some(v=>typeof v!=='string'||v.length>256))throw new Error('Invalid viewer identity');
    next.set(pair[0],pair[1]);
  }
  viewers.clear();for(const [pane,id] of next)viewers.set(pane,id);
}
export const managerViewerBound = (pane:string,id:string):boolean => viewers.get(pane)===id;
