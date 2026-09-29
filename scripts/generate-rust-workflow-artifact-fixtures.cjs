#!/usr/bin/env node
// Execute the shipping watcher parsers. No filesystem watchers or model calls.
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.resolve(__dirname,'..');
const ts=require(require.resolve('typescript',{paths:[path.join(root,'apps/desktop')]}));
const source=fs.readFileSync(path.join(root,'apps/desktop/src/main/services/workflowWatcher.ts'),'utf8');
const now=1767225600999;
class FixedDate extends Date { static now(){return now;} }
const sandbox={exports:{},Date:FixedDate,require(name){if(name==='./modelUsage')return {turnCostUSD(){throw Error('Artifact parser unexpectedly requested live usage');}};return require(name);}};
vm.runInNewContext(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,sandbox);
const watcher=sandbox.exports.workflowWatcher;
const samples=[
[{role:'user',content:'  first  '},{type:'assistant',timestamp:'2026-01-01T00:00:00Z',message:{content:[{type:'text',text:'hello'},{type:'tool_use',id:'x',name:'Read',input:{path:'a'}},{type:'tool_use',id:'interrupted',name:'Bash'}]}},{type:'user',timestamp:'2026-01-01T00:00:01Z',message:{content:[{type:'tool_result',tool_use_id:'x',content:[{text:'line1'},{type:'image'},{text:'line2'}],is_error:true},{type:'text',text:' next '}]} }],
[{role:'assistant',content:[{type:'tool_use'},{type:'tool_result',tool_use_id:'missing',content:'ignored in rich view'},{type:'text',text:'   '}]}],
[{type:'user',message:{content:[{type:'tool_result',content:'x'.repeat(401)}]}},{type:'assistant',message:{content:'🧑‍💻 Unicode'}},{type:'system',message:{content:'not a visible turn'}}],
[{type:'assistant',message:{content:[{type:'tool_use',id:'same',name:'One'},{type:'tool_use',id:'same',name:'Two'}]}},{type:'user',message:{content:[{type:'tool_result',tool_use_id:'same',content:'last duplicate wins'}]}}],
[]];
const rows=samples.map(sample=>{const raw='malformed JSON\n\n'+sample.map(row=>JSON.stringify(row)).join('\n');return{raw,now,transcript:watcher.parseTranscript(raw),conversation:watcher.parseConversation(raw)};});
const output=JSON.stringify(rows,null,2)+'\n';const target=path.join(root,'services/hub-rs/tests/fixtures/workflow-artifacts.json');
if(process.argv.includes('--check')){if(fs.readFileSync(target,'utf8')!==output)throw Error('Rust workflow artifact fixture is stale');}else fs.writeFileSync(target,output);
