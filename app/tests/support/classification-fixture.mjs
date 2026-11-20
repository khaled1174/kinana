import {readFileSync, writeFileSync, mkdirSync, chmodSync} from 'node:fs';
import path from 'node:path';
export function classificationFixture(base, repo) {
 const bin=path.join(base,'classification-bin'),trace=path.join(base,'classification-trace.jsonl'),hold=path.join(base,'hold-classification');
 mkdirSync(bin,{recursive:true});
 const help=readFileSync(path.join(repo,'docs/evidence/phase-3/claude-help.txt'),'utf8');
 const proposal={category:'design-ui',stack:['react'],stage:['review'],tags:['review'],role:'Review interface design',when_to_use:'When reviewing interface spacing and color',when_not:'For backend database work',improved_description:'Review interface spacing, typography, color, and hierarchy before changing a screen. Use for interface design reviews; skip backend database work.',overlaps_with:[],confidence:0.9};
 const script=`#!/usr/bin/python3
import json,os,sys,time
args=sys.argv[1:]
if args==['--help']:
 print(${JSON.stringify(help)})
 sys.exit(0)
if args==['--version']:
 print('2.1.323 (Claude Code)')
 sys.exit(0)
prompt=sys.stdin.read()
with open(${JSON.stringify(trace)},'a') as f:
 f.write(json.dumps({'kind':'model','pid':os.getpid(),'prompt_bytes':len(prompt.encode()),'fixture':True})+'\\n')
print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}),flush=True)
while os.path.exists(${JSON.stringify(hold)}):time.sleep(0.02)
time.sleep(0.7)
proposal=json.loads(${JSON.stringify(JSON.stringify(proposal))})
with open(${JSON.stringify(trace)}) as f:
 proposal['role']+=' %d'%len(f.readlines())
print(json.dumps({'type':'result','subtype':'success','is_error':False,'structured_output':proposal}))
`;
 const executable=path.join(bin,'claude');writeFileSync(executable,script);chmodSync(executable,0o700);
 return {bin,trace,hold,uses_paid_provider:false};
}
