/** Fixed private lifecycle callbacks over brain-owned stdio; never hub RPCs. */
let sequence = 0;
const pending = new Map<string,{resolve:(value:any)=>void;reject:(error:Error)=>void;timer:ReturnType<typeof setTimeout>}>();
export function hostCall<T = any>(method: string, params: unknown = {}): Promise<T> {
  if (pending.size >= 128) return Promise.reject(new Error('Too many pending lifecycle callbacks'));
  return new Promise((resolve,reject)=>{
    const hostCallId=String(++sequence);
    const timer=setTimeout(()=>{pending.delete(hostCallId);reject(new Error('Lifecycle acknowledgement timed out; outcome may be unknown'));},60_000);
    pending.set(hostCallId,{resolve,reject,timer});
    process.stdout.write(JSON.stringify({hostCallId,method,params})+'\n');
  });
}
export function acceptHostResult(frame: any): boolean {
  if (typeof frame.hostResultId !== 'string') return false;
  const entry=pending.get(frame.hostResultId);
  if (entry) {pending.delete(frame.hostResultId);clearTimeout(entry.timer);frame.error ? entry.reject(new Error(frame.error)) : entry.resolve(frame.result);}
  return true;
}
