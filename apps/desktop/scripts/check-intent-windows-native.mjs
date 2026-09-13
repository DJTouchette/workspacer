#!/usr/bin/env node
// Compile the embedded Windows helper without executing Win32 APIs. Useful on
// non-Windows development hosts; actual handle semantics require the Windows CI lane.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { buildSync } from 'esbuild';
const require = createRequire(import.meta.url);
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-windows-compile-'));
try {
  const version = execFileSync('dotnet', ['--version'], { encoding: 'utf8' }).trim().split('.')[0];
  if (!/^\d+$/.test(version)) throw new Error('Could not determine installed .NET SDK');
  const bundle = path.join(directory, 'source.cjs');
  buildSync({
    entryPoints: ['src/main/services/intentWindowsFiles.ts'],
    bundle: true,
    platform: 'node',
    outfile: bundle,
  });
  fs.writeFileSync(path.join(directory, 'Native.cs'), require(bundle).INTENT_WINDOWS_NATIVE_SOURCE);
  fs.writeFileSync(
    path.join(directory, 'Native.csproj'),
    `<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net${version}.0</TargetFramework><LangVersion>5</LangVersion><EnableDefaultCompileItems>false</EnableDefaultCompileItems></PropertyGroup><ItemGroup><Compile Include="Native.cs" /></ItemGroup></Project>`,
  );
  execFileSync(
    'dotnet',
    ['build', path.join(directory, 'Native.csproj'), '--nologo', '--verbosity', 'quiet'],
    { stdio: 'inherit' },
  );
} finally {
  fs.rmSync(directory, { recursive: true, force: true });
}
