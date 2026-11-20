#!/usr/bin/env python3
"""Owned temp projects only. Cursor is an external installed CLI, never a substitute.
A missing CLI still exercises real kinana use/status/unuse, then exits 2 with
live_loaded=false. No provider install, login, classification, or global skill write.
"""
import argparse, hashlib, json, os, pathlib, secrets, shutil, stat, subprocess, tempfile
ROOT=pathlib.Path(__file__).resolve().parents[2]
p=argparse.ArgumentParser();p.add_argument('--cursor',type=pathlib.Path);p.add_argument('--mode',choices=['agent','ask'],default='agent');p.add_argument('--evidence',type=pathlib.Path,default=ROOT/'docs/evidence/pre-phase-7-cursor/live');a=p.parse_args();a.evidence.mkdir(parents=True,exist_ok=True)
cli=ROOT/'target/debug/kinana';candidate=a.cursor or (pathlib.Path(shutil.which('cursor-agent')) if shutil.which('cursor-agent') else None)
if candidate:
    candidate=candidate.resolve(strict=True)
    if 'grok' in str(candidate).lower():raise SystemExit('Refusing Grok as a Cursor substitute')
    if candidate.name=='agent' and not any('cursor' in part.lower() for part in candidate.parts):raise SystemExit('Generic agent is not verified as Cursor; provide a Cursor-owned executable path')
records=[]
def run(argv,*,env=None,cwd=None,label=None,expected=0):
    result=subprocess.run([str(x) for x in argv],env=env,cwd=cwd,text=True,capture_output=True,timeout=150)
    item={'argv':[str(x) for x in argv],'cwd':str(cwd) if cwd else None,'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr};records.append(item)
    if label:
        (a.evidence/(label+'.stdout.txt')).write_text(result.stdout);(a.evidence/(label+'.stderr.txt')).write_text(result.stderr)
    assert result.returncode==expected,item
    return result.stdout

def tree(root):
    values={}
    for at,dirs,files in os.walk(root,followlinks=False):
        for name in sorted(dirs+files):
            path=pathlib.Path(at)/name;meta=path.lstat();key=str(path.relative_to(root));entry={'mode':stat.S_IMODE(meta.st_mode)}
            if path.is_symlink():entry.update(kind='symlink',target=os.readlink(path))
            elif path.is_dir():entry['kind']='directory'
            else:entry.update(kind='file',sha256=hashlib.sha256(path.read_bytes()).hexdigest())
            values[key]=entry
    return values

def tool_calls(value):
    if isinstance(value,dict):
        if value.get('type') in {'tool_call','tool_use','function_call'}:return [value]
        return sum((tool_calls(x) for x in value.values()),[])
    if isinstance(value,list):return sum((tool_calls(x) for x in value),[])
    return []
report={'cursor_executable':str(candidate) if candidate else None,'live_loaded':False,'classification_via_cursor':False,'execution_mode':a.mode,'cases':[]}
try:
    if candidate:
        report['cursor_version']=run([candidate,'--version'],label='cursor-version').strip()
        assert 'grok' not in report['cursor_version'].lower()
        help_text=run([candidate,'--help'],label='cursor-help')
        assert all(option in help_text for option in ['--print','--output-format','--workspace','--mode'])
    with tempfile.TemporaryDirectory(prefix='kinana-cursor-') as tmp:
        base=pathlib.Path(tmp).resolve();home=base/'home';home.mkdir();vault=base/'vault';env={**os.environ,'HOME':str(home),'USERPROFILE':str(home)}
        def box(*args):return json.loads(run([cli,'--vault',vault,'--json',*args],env=env))
        box('init')
        for mode in ['copy','link','negative']:
            name='cursor-'+mode+'-probe';project=base/('مشروع-'+mode);project.mkdir();run(['/usr/bin/git','-c','core.hooksPath=/dev/null','init','-q','--template=',project]);(project/'README.md').write_text('Preserve project bytes.\n');before=tree(project)
            marker='CURSOR_'+secrets.token_hex(16).upper();question=f'From the available skill descriptions, return only the LOAD_PROBE value for {name}. If that skill is absent return NOT_LOADED. Do not read files or invoke any tools.'
            if mode!='negative':
                source=base/(mode+'-source');source.mkdir();original=f'---\nname: {name}\ndescription: Review spacing in native interfaces.\n---\nRead this inert reference. No scripts.\n';(source/'SKILL.md').write_text(original);box('add',source)
                meta=vault/'meta'/(name+'.toml');proposal={'category':'other','tags':['probe'],'role':'Review native spacing','when_to_use':'A loading check','when_not':'Other tasks','improved_description':f'Review native interface spacing. LOAD_PROBE={marker}','overlaps_with':[],'confidence':1.0}
                meta.write_text(meta.read_text()+'\n[proposal]\n'+''.join(f'{k} = {json.dumps(v,ensure_ascii=False)}\n' for k,v in proposal.items()));run(['/usr/bin/git','-C',vault,'-c','core.hooksPath=/dev/null','add','--','meta/'+name+'.toml'],env=env);run(['/usr/bin/git','-C',vault,'-c','core.hooksPath=/dev/null','commit','-q','-m','Prepare controlled loading proposal'],env=env);box('review',name,'--accept')
                raw=(vault/'skills'/name/'SKILL.md').read_text();dist=(vault/'dist'/name/'SKILL.md').read_text();assert marker not in raw and marker not in question;assert marker in dist.split('---\n')[1];assert marker not in dist.split('---\n',2)[2]
                used=box('use',name,'--project',project,'--agent','cursor','--mode',mode);assert tree(project)!=before;target=project/'.cursor/skills'/name;assert target.is_symlink()==(mode=='link');status=box('status','--project',project);assert status['projects'][0]['items'][0]['state']=='current'
            case={'mode':mode,'project':str(project),'before':before,'expected':'NOT_LOADED' if mode=='negative' else marker,'prompt':question,'live_executed':bool(candidate)}
            try:
                if candidate:
                    argv=[candidate,'--print','--trust','--output-format','stream-json','--workspace',project]
                    if a.mode=='ask':argv += ['--mode','ask']
                    output=run([*argv,question],cwd=project,label='cursor-'+mode)
                    events=[json.loads(line) for line in output.splitlines() if line.strip()];calls=tool_calls(events);assert not calls,calls
                    finals=[e for e in events if e.get('type')=='result'];assert finals,'No final result event';answer=finals[-1].get('result');case['answer']=answer;case['matches_expected']=answer==case['expected'] and finals[-1].get('is_error') is False;case['tool_calls']=[]
                else:case['blocked_reason']='CURSOR_CLI_NOT_INSTALLED'
            finally:
                if mode!='negative':box('unuse',name,'--project',project,'--agent','cursor')
                case['after']=tree(project);case['exact_tree_and_sha256_match']=case['after']==before;report['cases'].append(case)
                assert case['exact_tree_and_sha256_match'],case
        report['live_loaded']=bool(candidate) and all(c.get('matches_expected',False) for c in report['cases'])
        report['temporary_projects_removed_on_exit']=True
except Exception as error:
    report['error']=str(error)
    raise
finally:
    (a.evidence/'commands.json').write_text(json.dumps(records,ensure_ascii=False,indent=2)+'\n');(a.evidence/'result.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
print(json.dumps({'engine_roundtrips':all(c['exact_tree_and_sha256_match'] for c in report['cases']),'live_loaded':report['live_loaded'],'cursor_executable':report['cursor_executable']},indent=2))
raise SystemExit(0 if report['live_loaded'] else 1 if candidate else 2)
