import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
for (const [binary,args,cwd,env] of [
  [process.execPath,['--test','scripts/test-desktop-host.mjs','scripts/test-headless-analytics.mjs'],desktop,process.env],
  ['go',['test','./cmd/brain','-run','TestDesktopHostEndToEnd|TestHeadlessManagerReplacementEndToEnd','-v'],path.resolve(desktop,'../../services/hub'),{...process.env,WKS_DESKTOP_HOST_TEST_BUNDLE:path.join(desktop,'dist/headless/desktop-host.cjs')}],
]) {
 const result=spawnSync(binary,args,{cwd,env,stdio:'inherit'});
 if(result.error)throw result.error;
 if(result.status!==0)process.exit(result.status??1);
}
