import {copyFileSync, existsSync, mkdirSync, renameSync, writeFileSync, readFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {prepareSourceFixture} from './scripts/source-ui-fixture.mjs';
import {isolatedSession, e2eService, focusTestWindow, prepareTestDriver} from './tests/support/isolated-ui.mjs';
import {syntheticSkill} from './tests/support/synthetic-skill.mjs';
import {classificationFixture} from './tests/support/classification-fixture.mjs';

const session = isolatedSession();
const {base, home, evidence, cli} = session;
const agentCase = process.env.KINANA_UI_AGENT_HOME_CASE;
const repo = fileURLToPath(new URL('../', import.meta.url));
const fixturePath = path.join(base, 'general-ui-fixture.json');
if (!existsSync(fixturePath)) {
  const vault = path.join(home, 'SkillsBox');
  const seed = classificationFixture(path.join(base, 'seed'), repo);
  const provider = classificationFixture(base, repo);
  const env = {...process.env, PATH: seed.bin + path.delimiter + process.env.PATH};
  const run = args => {
    const result = spawnSync(cli, ['--vault', vault, '--json', ...args], {env, encoding: 'utf8'});
    if (result.status !== 0) throw new Error(result.stderr || result.stdout);
  };
  const names = ['emil-color', 'emil-design-foundations', 'emil-typography', 'emil-ui-polish',
    'emil-ui-review', 'emil-component-design', 'emil-design-system-docs'];
  const sources = path.join(base, 'sources');
  for (const name of names) syntheticSkill(path.join(sources, name), name,
    name === 'emil-typography' ? 'Typography for headings and body text.' : undefined);
  if (!process.env.KINANA_UI_EMPTY) {
    run(['init']);
    // r5 (owner, 2026-11-24): classification approves clean skills'
    // descriptions by default. These specs cover the manual review, so the
    // seed vault keeps it off; the classification spec tests the approval.
    {
      const box = path.join(vault, 'box.toml');
      writeFileSync(box, 'auto_approve_descriptions = false\n' + readFileSync(box, 'utf8'));
      for (const args of [['add', '--', 'box.toml'], ['commit', '-m', 'Review descriptions by hand']]) {
        const result = spawnSync('/usr/bin/git', ['-C', vault, ...args], {env, encoding: 'utf8'});
        if (result.status !== 0) throw new Error(result.stderr || result.stdout);
      }
    }
    for (const name of names) run(['add', path.join(sources, name)]);
    run(['classify', '--all', '--agent', 'keywords', '--yes']);
    run(['classify', 'emil-design-foundations', '--agent', 'claude', '--yes']);
    if (process.env.KINANA_ISOLATED_SPEC === 'batch-review') {
      run(['classify', 'emil-ui-review', '--agent', 'claude', '--yes']);
    }
    if (agentCase === 'missing') {
      const box = path.join(vault, 'box.toml');
      const config = readFileSync(box, 'utf8');
      if (!config.includes('classification_agent = "claude"')) throw new Error('Unexpected seed classifier default');
      writeFileSync(box, config.replace('classification_agent = "claude"', 'classification_agent = "auto"'));
      for (const args of [['add', '--', 'box.toml'], ['commit', '-m', 'Use auto in the missing-agent fixture']]) {
        const result = spawnSync('/usr/bin/git', ['-C', vault, ...args], {env, encoding: 'utf8'});
        if (result.status !== 0) throw new Error(result.stderr || result.stdout);
      }
    }
    if (process.env.KINANA_UI_LEGACY) renameSync(path.join(home, '.kinana'), path.join(home, '.skillsbox'));
  }
  const source = prepareSourceFixture(base);
  const fixture = {source: path.join(source.root, 'fixture.json'), bin: source.bin, provider: provider.bin};
  writeFileSync(fixturePath, JSON.stringify(fixture));
  writeFileSync(path.join(evidence, 'ui-fixture-provenance.json'), JSON.stringify({home, vault, sources,
    method: 'synthetic skills and fake provider'}, null, 2) + '\n');
  if (agentCase === 'outside-path') {
    const localBin = path.join(home, '.local/bin');
    mkdirSync(localBin, {recursive: true});
    copyFileSync(path.join(provider.bin, 'claude'), path.join(localBin, 'claude'));
    writeFileSync(path.join(evidence, 'agent-home-outside-path.json'), JSON.stringify({trace: provider.trace}));
  }
}
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8'));
process.env.KINANA_UI_SOURCE_FIXTURE = fixture.source;
process.env.KINANA_UI_AGENT_PATH = agentCase ? '/usr/bin:/bin:/usr/sbin:/sbin'
  : [fixture.bin, fixture.provider, process.env.PATH].join(path.delimiter);
export const config={
 runner:'local',specs:['./tests/shell.e2e.mjs','./tests/vault.e2e.mjs','./tests/projects.e2e.mjs','./tests/settings.e2e.mjs','./tests/acceptance.e2e.mjs','./tests/cursor.e2e.mjs','./tests/source-updates.e2e.mjs','./tests/vault-location.e2e.mjs'],maxInstances:1,
 logLevel:'warn',outputDir:evidence,framework:'mocha',reporters:['spec'],
 before:prepareTestDriver,beforeTest:focusTestWindow,
 mochaOpts:{timeout:180000},connectionRetryCount:0,
 services:[e2eService(session,{PATH:process.env.KINANA_UI_AGENT_PATH})],
 capabilities:[{browserName:'tauri'}],
 async afterTest(test,context,{error}){if(error){const name=test.title.replace(/[^a-z0-9]+/gi,'-').slice(0,80);await browser.saveScreenshot(path.resolve(evidence,name+'.png'));const dom=await browser.execute(()=>({errors:Array.from(document.querySelectorAll('[role="alert"]')).map(e=>e.textContent),dialogs:Array.from(document.querySelectorAll('[role="dialog"]')).map(e=>e.textContent),search:document.querySelector('[data-testid="search"]')?.value,visibility:document.visibilityState,styles:Array.from(document.querySelectorAll('[data-testid="classify-estimate"],[data-slot="dialog-content"],[data-slot="dialog-overlay"]')).map(e=>{const c=getComputedStyle(e),r=e.getBoundingClientRect();return{slot:e.dataset.slot,testid:e.dataset.testid,opacity:c.opacity,display:c.display,visibility:c.visibility,animation:c.animation,rect:{x:r.x,y:r.y,width:r.width,height:r.height}}})}));writeFileSync(path.resolve(evidence,name+'.json'),JSON.stringify(dom,null,2));}},
 onComplete(){const result=path.join(evidence,'onboarding-result.json');const vault=process.env.KINANA_UI_EMPTY?(existsSync(result)?JSON.parse(readFileSync(result,'utf8')).vault:null):path.join(home,'SkillsBox');writeFileSync(path.join(evidence,process.env.KINANA_UI_EMPTY?'onboarding-session.json':'ui-session.json'),JSON.stringify({home,vault,native:true,mocked:false},null,2)+'\n');},
};
