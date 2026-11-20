import assert from 'node:assert/strict';
import {test} from 'node:test';
import {readFileSync,readdirSync,statSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {classificationOutcome,itemReason} from '../src/lib/classification.ts';
import {reasonText} from '../src/reasons.ts';

const app=fileURLToPath(new URL('..',import.meta.url));
const skill=(id,reason_code,classified_by='keywords',proposal_created=false)=>({id,classified_by,proposal_created,reason_code});

test('one cause shared by every fallback is shown once, not as a generic failure',()=>{
 const items=Array.from({length:45},(_,i)=>skill(`s${i}`,'AGENT_EXECUTABLE_NOT_FOUND'));
 const outcome=classificationOutcome(items);
 assert.deepEqual(outcome,{fallback:true,allFallback:true,sharedReason:'AGENT_EXECUTABLE_NOT_FOUND'});
 assert.equal(itemReason(items[0],outcome),undefined);
 assert.match(reasonText(outcome.sharedReason,'ar'),/~\/\.local\/bin/);
 assert.match(reasonText(outcome.sharedReason,'ar'),/لم يُشغَّل Claude/);
});

test('mixed causes are named next to each skill; proposals carry none',()=>{
 const items=[skill('a','AGENT_TOOLS_NOT_ISOLATED'),skill('b','AGENT_TIMEOUT'),skill('c',undefined,'claude',true)];
 const outcome=classificationOutcome(items);
 assert.deepEqual(outcome,{fallback:true,allFallback:false,sharedReason:undefined});
 assert.deepEqual(items.map(item=>itemReason(item,outcome)),['AGENT_TOOLS_NOT_ISOLATED','AGENT_TIMEOUT',undefined]);
 assert.match(reasonText('AGENT_TOOLS_NOT_ISOLATED','ar'),/إضافات Claude Code/);
 assert.match(reasonText('AGENT_TOOLS_NOT_ISOLATED','ar'),/العزل/);
});

test('every fallback code the core emits has its own translated reason',()=>{
 const core=readFileSync(path.join(app,'../crates/core/src/agent_runner.rs'),'utf8');
 const codes=new Set([...core.matchAll(/"((?:AGENT_[A-Z_]+)|INVALID_AGENT_PROPOSAL)"/g)].map(m=>m[1]));
 codes.delete('AGENT_USAGE_LIMIT');// a stop, reported by the stopped banner, never a per-skill fallback
 assert.ok(codes.has('AGENT_EXECUTABLE_NOT_FOUND')&&codes.has('AGENT_TOOLS_NOT_ISOLATED'),[...codes].join());
 const unknown=reasonText('NO_SUCH_CODE','ar');
 for(const code of codes)for(const language of ['ar','en']){
  const text=reasonText(code,language);
  assert.notEqual(text,reasonText('NO_SUCH_CODE',language),code);
  assert.doesNotMatch(text,/تعذّر التصنيف عبر الوكيل|Agent classification failed/,code);
 }
 assert.ok(unknown);
});

test('the interface never shows development paths such as docs/ or the calibration log',()=>{
 const files=[];
 const walk=directory=>{for(const name of readdirSync(directory)){const file=path.join(directory,name);if(statSync(file).isDirectory())walk(file);else if(/\.(tsx?|css|html)$/.test(name))files.push(file);}};
 walk(path.join(app,'src'));
 assert.ok(files.length>10);
 for(const file of files){
  const text=readFileSync(file,'utf8'),name=path.relative(app,file);
  assert.doesNotMatch(text,/["'`/]docs\//,`${name} contains a docs/ path`);
  // bridge.ts only declares the field so the estimate can be sent back unchanged.
  if(!name.endsWith('bridge.ts'))assert.doesNotMatch(text,/source_log/,`${name} reads the calibration source log`);
 }
});
