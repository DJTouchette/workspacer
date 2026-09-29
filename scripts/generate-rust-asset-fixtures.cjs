#!/usr/bin/env node
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const root=path.resolve(__dirname,'..');const ts=require(require.resolve('typescript',{paths:[path.join(root,'apps/desktop')]}));
function load(relative){const context={exports:{}};vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(root,relative),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);return context.exports;}
const fonts=load('apps/desktop/src/main/shared/customFonts.ts'),headers=load('apps/desktop/src/main/services/imageHeader.ts');
const buffers=[];let b=Buffer.alloc(24);Buffer.from('89504e470d0a1a0a','hex').copy(b);b.write('IHDR',12);b.writeUInt32BE(320,16);b.writeUInt32BE(200,20);buffers.push(b);
buffers.push(Buffer.from('GIF89a\x02\0\x03\0','binary'));
b=Buffer.alloc(26);b.write('BM');b.writeInt32LE(100,18);b.writeInt32LE(-200,22);buffers.push(b);
for(const type of ['VP8 ','VP8L','VP8X']){b=Buffer.alloc(30);b.write('RIFF');b.write('WEBP',8);b.write(type,12);if(type==='VP8 '){b.writeUInt16LE(321,26);b.writeUInt16LE(222,28);}if(type==='VP8L')b.writeUInt32LE((222-1)<<14|(321-1),21);if(type==='VP8X'){b.writeUIntLE(320,24,3);b.writeUIntLE(221,27,3);}buffers.push(b);}
b=Buffer.alloc(22);b.writeUInt16BE(0xffd8);b[2]=0xff;b[3]=0xc0;b.writeUInt16BE(222,7);b.writeUInt16BE(321,9);buffers.push(b);buffers.push(Buffer.from('not-an-image'));
const output=JSON.stringify({fonts:['JetBrainsMono-VariableFont[wght].woff2','Fira_Code-Regular.ttf','Inter[opsz,wght]-VF.otf','IBM.Plex.Variable.woff','Regular.ttf','name_with.dots.ttf','Fancy-VariableFont_test.OTF','Sans.ttf'].map(file=>({file,family:fonts.customFontFamily(file)})),headers:buffers.map(b=>({base64:b.toString('base64'),dimensions:headers.readImageDimensions(b)}))},null,2)+'\n';const target=path.join(root,'services/hub-rs/tests/fixtures/ui-assets.json');if(process.argv.includes('--check')){if(fs.readFileSync(target,'utf8')!==output)throw Error('UI asset fixture stale');}else fs.writeFileSync(target,output);
