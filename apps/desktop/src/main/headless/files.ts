import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { assertPathAllowed } from '../lib/pathConfinement';

/** Owner download, using the same workspace and sensitive-path guard as edits. */
export function readFileBytes(requested: unknown, roots: string[]) {
  if (typeof requested !== 'string') throw new Error('Invalid file path');
  const file=assertPathAllowed('desktop.readFileBytes',requested,roots);
  const limit=16*1024*1024;
  const fd=fs.openSync(file,fs.constants.O_RDONLY|(fs.constants.O_NOFOLLOW??0));
  try {
    const inspected=fs.fstatSync(fd),current=fs.statSync(file);
    if(!inspected.isFile()||inspected.size>limit||inspected.ino!==current.ino||inspected.dev!==current.dev||assertPathAllowed('desktop.readFileBytes',file,roots)!==file)throw new Error('Choose an unchanged regular file up to 16 MiB');
    const bytes=Buffer.alloc(limit+1);let size=0;
    while(size<bytes.length){const n=fs.readSync(fd,bytes,size,bytes.length-size,null);if(!n)break;size+=n;}
    if(size>limit)throw new Error('File exceeds 16 MiB');
    return {name:path.basename(file),dataBase64:bytes.subarray(0,size).toString('base64')};
  } finally {fs.closeSync(fd);}
}

/** Native-picker equivalent: filenames only, under the server user's OS rights. */
export function listPickerEntries(requested: unknown) {
  if(requested!==undefined&&typeof requested!=='string')throw new Error('Invalid directory');
  const initial=typeof requested==='string'&&requested ? requested.replace(/^~(?=[/\\]|$)/,os.homedir()) : os.homedir();
  const resolved=fs.realpathSync(initial);
  const directory=fs.statSync(resolved).isDirectory()?resolved:path.dirname(resolved);
  const entries=fs.readdirSync(directory,{withFileTypes:true});
  if(entries.length>10_000)throw new Error('Directory has too many entries; choose a narrower path');
  const files=entries.map(entry=>{
    const target=path.join(directory,entry.name);
    let isDir=entry.isDirectory();
    if(entry.isSymbolicLink()){try{isDir=fs.statSync(target).isDirectory()}catch{}}
    return {name:entry.name,path:target,isDir};
  }).sort((a,b)=>Number(b.isDir)-Number(a.isDir)||a.name.localeCompare(b.name));
  return {path:directory,parent:path.dirname(directory),home:os.homedir(),entries:files};
}
