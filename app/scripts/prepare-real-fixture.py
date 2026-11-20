from pathlib import Path
import tempfile,subprocess,os,json,hashlib
root=Path(__file__).resolve().parents[2];base=Path(tempfile.mkdtemp(prefix='kinana-phase62-',dir=Path(tempfile.gettempdir()).resolve()))
home=base/'home';home.mkdir();vault=home/'SkillsBox';cli=root/'target/debug/kinana';e=root/'docs/evidence/phase-6/batch-2b'
context={'base':str(base),'home':str(home),'vault':str(vault),'sources':[]};(root/'.git/phase62-live-fixture.json').write_text(json.dumps(context,indent=2)+'\n')
env={**os.environ,'HOME':str(home)}
def run(args):
 cmd=[str(cli),'--vault',str(vault),'--json',*args];p=subprocess.run(cmd,env=env,text=True,capture_output=True)
 with (e/'real-fixture-commands.jsonl').open('a') as out:out.write(json.dumps({'argv':cmd,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr},ensure_ascii=False)+'\n')
 print(' '.join(args),p.returncode,flush=True)
 if p.returncode:raise RuntimeError(p.stdout+p.stderr)
 return json.loads(p.stdout)
run(['init'])
for name in ['emil-color','emil-design-foundations','emil-forms-and-inputs','emil-surfaces','emil-typography','emil-ui-polish','emil-ui-review']:
 source=Path.home()/'.codex/skills'/name
 context['sources'].append({'path':str(source),'skill_md_sha256':hashlib.sha256((source/'SKILL.md').read_bytes()).hexdigest()})
 run(['add',str(source),'--allow-risky'])
run(['classify','--all','--agent','keywords','--yes'])
# Keep provider authentication in the normal CLI environment, while core receives
# explicit temporary vault/index paths. No credentials are copied or printed.
cmd=['cargo','run','--locked','-q','-p','kinana-core','--example','classify-test-vault','--',str(vault),str(home/'.kinana/index.db'),'emil-design-foundations']
p=subprocess.run(cmd,cwd=root,text=True,capture_output=True)
with (e/'real-fixture-commands.jsonl').open('a') as out:out.write(json.dumps({'argv':cmd,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr},ensure_ascii=False)+'\n')
if p.returncode:raise RuntimeError(p.stderr)
report=json.loads(p.stdout);assert report['items'][0]['classified_by']=='claude' and report['items'][0]['proposal_created'],report
context['classified_skill']='emil-design-foundations'
context['baseline_commit']=subprocess.check_output(['git','-C',str(vault),'rev-parse','HEAD'],text=True).strip()
(root/'.git/phase62-live-fixture.json').write_text(json.dumps(context,indent=2)+'\n')
(e/'real-fixture-provenance.json').write_text(json.dumps(context,indent=2)+'\n')
print('REAL_FIXTURE_READY',base,flush=True)
