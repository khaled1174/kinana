import {mkdirSync,mkdtempSync,writeFileSync,chmodSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
export function prepareSourceFixture(base){
 const root=mkdtempSync(path.join(base,'source-fixture-')),repo=path.join(root,'upstream'),bin=path.join(root,'bin'),log=path.join(root,'network.jsonl');mkdirSync(repo);mkdirSync(bin);
 function git(args){const result=spawnSync('/usr/bin/git',['-C',repo,'-c','core.hooksPath=/dev/null','-c','user.name=Source UI Fixture','-c','user.email=test@localhost',...args],{encoding:'utf8'});if(result.status!==0)throw Error(result.stdout+result.stderr);return result.stdout.trim();}
 git(['init','-q','--template=','-b','main']);
 for(const name of ['source-update-probe','source-update-peer']){const dir=path.join(repo,'skills',name);mkdirSync(dir,{recursive:true});writeFileSync(path.join(dir,'SKILL.md'),`---\nname: ${name}\ndescription: Original source description for spacing reviews.\n---\n# Spacing reference\nReview the spacing rhythm.\n`);writeFileSync(path.join(dir,'reference.txt'),'Original reference.\n');}
 git(['add','--all']);git(['commit','-qm','Initial source fixtures']);
 const wrapper=path.join(bin,'git');writeFileSync(wrapper,`#!/usr/bin/python3\nimport json,os,sys\nargs=sys.argv[1:]\nfor op in ['clone','fetch']:\n if op in args and 'https://github.com/fixture/skills.git' in args:\n  with open(${JSON.stringify(log)},'a') as log:log.write(json.dumps({'operation':op,'args':args})+'\\n')\n  args=[${JSON.stringify(repo)} if x=='https://github.com/fixture/skills.git' else x for x in args]\n  pos=args.index(op);args[pos:pos]=['-c','protocol.file.allow=always']\nos.execv('/usr/bin/git',['git']+args)\n`);chmodSync(wrapper,0o700);
 const fixture={root,repo,bin,log,url:'https://github.com/fixture/skills/tree/main/skills',initial_commit:git(['rev-parse','HEAD'])};writeFileSync(path.join(root,'fixture.json'),JSON.stringify(fixture,null,2)+'\n');return fixture;
}
