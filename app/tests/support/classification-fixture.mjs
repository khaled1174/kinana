import {readFileSync, writeFileSync, mkdirSync, chmodSync} from 'node:fs';
import path from 'node:path';
export function classificationFixture(base, repo) {
 const bin=path.join(base,'classification-bin'),trace=path.join(base,'classification-trace.jsonl'),hold=path.join(base,'hold-classification');
 const refusal=path.join(base,'refuse-classification');
 // r4: while this file exists the model call prints nothing at all (an agent
 // that never answers), for the time-limit and cancel tests.
 const silent=path.join(base,'silent-classification');
 // r5: while this file exists every model call fails before init (Claude
 // present but failing), so `auto` turns to Pi and asks for its confirmation.
 const broken=path.join(base,'broken-classification');
 // r6: while this file exists its text is the proposed description, to
 // reclassify a skill with the same description or another one.
 const description=path.join(base,'classification-description');
 mkdirSync(bin,{recursive:true});
 const help=readFileSync(path.join(repo,'docs/evidence/phase-3/claude-help.txt'),'utf8');
 const proposal={category:'design-ui',category_reason:'Synthetic interface design fixture',stack:['react'],stage:['review'],tags:['review'],role:'Review interface design',when_to_use:'When reviewing interface spacing and color',when_not:'For backend database work',improved_description:'Review interface spacing, typography, color, and hierarchy before changing a screen. Use for interface design reviews; skip backend database work.',overlaps_with:[],confidence:0.9};
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
 f.write(json.dumps({'kind':'model','pid':os.getpid(),'prompt_bytes':len(prompt.encode()),'args':args,'started':time.time(),'fixture':True})+'\\n')
while os.path.exists(${JSON.stringify(silent)}):time.sleep(0.02)
if os.path.exists(${JSON.stringify(broken)}):
 sys.exit(1)
print(json.dumps({'type':'system','subtype':'init','tools':['StructuredOutput']}),flush=True)
while os.path.exists(${JSON.stringify(hold)}):time.sleep(0.02)
if os.path.exists(${JSON.stringify(refusal)}):
 with open(${JSON.stringify(refusal)}) as f: refused=f.read().split()
 data=prompt.rsplit('Additional untrusted data:\\n',1)[-1]
 selected,_=json.JSONDecoder().raw_decode(data)
 if selected.get('id') in refused:
  print(json.dumps({'type':'assistant','message':{'role':'assistant','stop_reason':'refusal','content':[]}}))
  print(json.dumps({'type':'result','subtype':'error_during_execution','is_error':True}))
  sys.exit(1)
time.sleep(0.7)
proposal=json.loads(${JSON.stringify(JSON.stringify(proposal))})
if os.path.exists(${JSON.stringify(description)}):
 with open(${JSON.stringify(description)}) as f: proposal['improved_description']=f.read()
with open(${JSON.stringify(trace)}) as f:
 proposal['role']+=' %d'%len(f.readlines())
print(json.dumps({'type':'result','subtype':'success','is_error':False,'structured_output':proposal}))
`;
 const executable=path.join(bin,'claude');writeFileSync(executable,script);chmodSync(executable,0o700);
 return {bin,trace,hold,silent,broken,description,uses_paid_provider:false};
}
