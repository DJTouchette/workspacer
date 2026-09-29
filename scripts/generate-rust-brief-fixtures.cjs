#!/usr/bin/env node
// Reference output from the shipping TypeScript board, never reimplemented here.
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.resolve(__dirname,'..');
const ts=require(require.resolve('typescript',{paths:[path.join(root,'apps/desktop')]}));
const source=fs.readFileSync(path.join(root,'apps/desktop/src/main/shared/briefBoard.ts'),'utf8');
const sandbox={exports:{}};
vm.runInNewContext(ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,sandbox);
const api=sandbox.exports;
const shared=JSON.parse(fs.readFileSync(path.join(root,'contracts/brief-board-cases.json'),'utf8'));
const samples=[...shared.entries.map(x=>x.brief),
'## Now\n- 🚧 **Fresh implementation** dispatched session:abcdef01 — in flight\n- 2026-08-22 ✅ **Completed implementation** — commit `123abcd`, session:12345678\n- ❌ WRONG **Misleading old headline** — RETRACTED SAME DAY\n- NOT YET DISPATCHED — next up\n- Bug FIXED and merged — discarded an earlier dispatched attempt\n- ✅ RESOLVED but ONE HUMAN ACTION NEEDED\n- ⚠️ Awaiting a decision\n- Design details and `code` only\n',
'## User\n- Personal standing preference\n## Now\n- Here right-click the card → **Terminate** → reopen from Overview\n- Long supplementary 🧑‍💻 headline '+ 'word '.repeat(70)+'\n- Keep ``double`` ticks and *emphasis* unchanged in the source\n',
'## Now\n- **Bold span with\n  a continuation** is underway\n- session:abcdef123456 and `1234567` plus `7654321` and `3456789` and `abcdef0` and `4567890`\n'];
const rows=samples.map(content=>({content,expected:api.cardsForBrief(content),ids:api.parseBrief(content).entries.map(e=>e.id)}));
const content='## Now\n- Fixed the client session:abcdef12\n';const id=api.parseBrief(content).entries[0].id;
const index={cards:{[id]:{title:' **Indexed replacement** ',status:'waiting_on_you',summary:'A clearer description',refs:['session:feedface',42]}}};
rows.push({content,index,expected:api.cardsForBrief(content,api.normalizeIndex(index)),ids:[id]});
const output=JSON.stringify(rows,null,2)+'\n';const target=path.join(root,'services/hub-rs/tests/fixtures/brief-cards.json');
if(process.argv.includes('--check')){if(fs.readFileSync(target,'utf8')!==output)throw Error('Rust brief reference fixture is stale');}else fs.writeFileSync(target,output);

const loaded=new Map();
function loadTs(filename){filename=path.resolve(filename);if(loaded.has(filename))return loaded.get(filename);const context={exports:{},require:(spec)=>{if(!spec.startsWith('.'))return require(spec);return loadTs(path.resolve(path.dirname(filename),spec+'.ts'));}};loaded.set(filename,context.exports);vm.runInNewContext(ts.transpileModule(fs.readFileSync(filename,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);return context.exports;}
const checker=loadTs(path.join(root,'apps/desktop/src/main/services/briefCheck.ts'));
const reports=shared.check.map(row=>({...row,expected:checker.checkNowSection(row.brief,checker.liveSessionIds(row.sessions),'fixture')}));
const reportOutput=JSON.stringify(reports,null,2)+'\n';const reportPath=path.join(root,'services/hub-rs/tests/fixtures/brief-reports.json');
if(process.argv.includes('--check')){if(fs.readFileSync(reportPath,'utf8')!==reportOutput)throw Error('Rust brief report fixture is stale');}else fs.writeFileSync(reportPath,reportOutput);
