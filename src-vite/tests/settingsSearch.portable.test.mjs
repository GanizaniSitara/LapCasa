// Run the browser suite with workstation-external test drivers forbidden.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

test('Settings browser tests run using only repository-owned harness code', () => {
  const script = `
    import child from 'node:child_process';
    import {syncBuiltinESMExports} from 'node:module';
    import {fileURLToPath} from 'node:url';
    import path from 'node:path';
    const root=${JSON.stringify(fileURLToPath(new URL('.', import.meta.url)))};
    const spawn=child.spawnSync;
    child.spawnSync=(command,args,options)=>{
      if(!path.resolve(args[0]).startsWith(root))throw Error('External browser harness forbidden: '+args[0]);
      return spawn(command,args,options);
    };
    syncBuiltinESMExports();
    await import(${JSON.stringify(new URL('./settingsSearch.test.mjs', import.meta.url).href)});
  `;
  const env={...process.env};delete env.NODE_TEST_CONTEXT;
  const result=spawnSync(process.execPath,['--input-type=module','--eval',script],{env,encoding:'utf8',timeout:90000});
  assert.equal(result.status,0,result.stdout+result.stderr);
});
