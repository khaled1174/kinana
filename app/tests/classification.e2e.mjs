import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,readdirSync,lstatSync,unlinkSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
const home=process.env.KINANA_UI_HOME,vault=path.join(home,'KinanaVault'),evidence=process.env.KINANA_UI_EVIDENCE;
const trace=()=>existsSync(process.env.KINANA_CLASSIFY_TRACE)?readFileSync(process.env.KINANA_CLASSIFY_TRACE,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
function cli(args){const p=spawnSync(process.env.KINANA_TEST_CLI,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home,PATH:process.env.KINANA_CLASSIFY_BIN+path.delimiter+process.env.PATH},encoding:'utf8'});assert.equal(p.status,0,p.stdout+p.stderr);return JSON.parse(p.stdout);}
function tree(root){const result={};function visit(at){for(const name of readdirSync(at).sort()){const p=path.join(at,name);if(lstatSync(p).isDirectory())visit(p);else result[path.relative(root,p)]=createHash('sha256').update(readFileSync(p)).digest('hex');}}visit(root);return result;}
async function invoke(command,params={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),command,params);}
const ids=['emil-color','emil-design-foundations'];
let baseline,approved,baseCalls;
describe('Classify every vault skill through native core',()=>{
 it('offers the action in first run, previews every skill, and cancels without provider calls',async()=>{
  await $('[data-testid="onboarding"]').waitForDisplayed();if(await $('html').getAttribute('lang')!=='ar')await $('[data-testid="language"]').click();
  await $('[data-testid="settings-initialize"]').click();await $('[data-testid="settings-preview"]').waitForDisplayed();await $('[data-testid="confirm-settings"]').click();await $('[data-testid="onboarding-next"]').waitForDisplayed();await $('[data-testid="onboarding-next"]').click();await $('[data-testid="onboarding-skip-scan"]').click();
  assert.equal(await $('[data-testid="settings-classify-all"]').isEnabled(),false);
  await $('button=السابق').click();
  assert.equal(await $('[data-testid="settings-scan-paths"]').isDisplayed(),false);
  const locations=await invoke('agent_skill_locations',{agents:['claude','codex','cursor']});
  assert.deepEqual(locations.roots,[path.join(home,'.codex/skills')]);assert.equal(locations.locations.length,4);
  assert.deepEqual(locations.locations.find(l=>l.path===path.join(home,'.agents/skills')).agents,['codex','cursor']);
  await browser.saveScreenshot(path.join(evidence,'automatic-scan-ar.png'));
  await $('[data-testid="language"]').click();await browser.saveScreenshot(path.join(evidence,'automatic-scan-en.png'));await $('[data-testid="language"]').click();
  await $('[data-testid="scan-agent-skills"]').click();await $('[data-testid="scan-results"]').waitForDisplayed();
  assert.match(await $('[data-testid="scan-counts"]').getText(),/جديدة: [2٢]/);
  assert.deepEqual(cli(['list']),[]);assert.equal(trace().length,0);
  await browser.saveScreenshot(path.join(evidence,'automatic-scan-results-ar.png'));
  await $('[data-testid="cancel-scan"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting());
  await $('[data-testid="scan-agent-locations"]').waitForDisplayed();await browser.saveScreenshot(path.join(evidence,'automatic-folders-ar.png'));
  for(const name of ['claude','codex','cursor'])await $(`[data-testid="scan-agent-${name}"]`).click();
  assert.equal(await $('[data-testid="scan-agent-skills"]').isEnabled(),false);
  await $('[data-testid="scan-agent-claude"]').click();await $('[data-testid="scan-agent-skills"]').click();await $('[data-testid="scan-agent-locations"]').waitForDisplayed();
  assert.equal(await $('.scan-dialog').isExisting(),false);assert.deepEqual(cli(['list']),[]);
  await $('[data-testid="scan-agent-codex"]').click();await $('[data-testid="scan-agent-skills"]').click();await $('[data-testid="scan-results"]').waitForDisplayed();await $('[data-testid="confirm-scan-import"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting());
  writeFileSync(path.join(evidence,'automatic-discovery.json'),JSON.stringify({locations,real_skill_fixtures:true,manual_path_required:false,read_only_before_confirmation:true,missing_roots_explained:true,paid_calls:0},null,2)+'\n');
  await $('[data-testid="settings-classify-all"]').waitForEnabled();
  cli(['classify','emil-color','--agent','claude','--yes']);cli(['review','emil-color','--accept']);
  approved=cli(['show','emil-color']).metadata.description;assert.equal(approved.active,'improved');
  baseline={meta:tree(path.join(vault,'meta')),dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills'))};baseCalls=trace().length;
  await browser.saveScreenshot(path.join(evidence,'onboarding-all-ar.png'));
  const single=await invoke('preview_classify',{id:'emil-color'});assert.equal(single.selected,1);assert.equal(single.estimated_agent_calls,1);
  const preview=await invoke('preview_classify',{all:true});assert.equal(preview.selected,2);assert.equal(preview.estimated_agent_calls,2);assert.equal(preview.items.length,2);
  const expected=cli(['classify','--all','--agent','claude','--dry-run']);assert.deepEqual(preview.estimated_tokens,expected.estimated_tokens);
  await $('[data-testid="settings-classify-all"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();assert.match(await $('[data-testid="classify-count"]').getText(),/2|٢/);await browser.saveScreenshot(path.join(evidence,'estimate-all-ar.png'));await $('[data-testid="cancel-classify"]').click();
  assert.deepEqual({meta:tree(path.join(vault,'meta')),dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills'))},baseline);assert.equal(trace().length,baseCalls);
  await $('[data-testid="language"]').click();await $('[data-testid="settings-classify-all"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();assert.match(await $('.classify-dialog').getText(),/Classify all skills/);await browser.saveScreenshot(path.join(evidence,'estimate-all-en.png'));await $('[data-testid="cancel-classify"]').click();await $('[data-testid="language"]').click();
  for(const [confirmed,estimate,code] of [[false,preview.estimated_tokens,'CLASSIFY_CONFIRMATION_REQUIRED'],[true,{...preview.estimated_tokens,total_input_tokens:preview.estimated_tokens.total_input_tokens+1},'CLASSIFY_ESTIMATE_CHANGED']]){
   await invoke('start_classify',{all:true,estimate,confirmed});await browser.waitUntil(async()=>(await invoke('classify_status')).done);assert.equal((await invoke('classify_status')).error.code,code);await invoke('cancel_classify');
  }
  assert.equal(trace().length,baseCalls);assert.deepEqual(tree(path.join(vault,'meta')),baseline.meta);
  writeFileSync(path.join(evidence,'preview.json'),JSON.stringify({preview,cli_estimate:expected.estimated_tokens,read_only:true,confirmation_and_stale_estimate_rejected:true,paid_calls:0},null,2)+'\n');
 });
 it('classifies the full batch with progress and keeps approved descriptions and dist unchanged',async()=>{
  await $('[data-testid="settings-classify-all"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classify-progress"]').waitForDisplayed();
  const samples=[];await browser.waitUntil(async()=>{if(await $('[data-testid="classification-result"]').isExisting())return true;const s=await invoke('classify_status');samples.push(s.progress);return false;},{timeout:30000,interval:80});
  assert.ok(samples.some(p=>p.total===2&&p.current_id));assert.equal(trace().length-baseCalls,2);
  assert.equal((await $('[data-testid="classification-items"]').$$('li')).length,2);
  const records=ids.map(id=>cli(['show',id]));
  for(const record of records){assert.equal(record.metadata.classification.classified_by,'claude');assert.ok(record.metadata.proposal);}
  assert.deepEqual(records[0].metadata.description,approved);assert.deepEqual(tree(path.join(vault,'dist')),baseline.dist);assert.deepEqual(tree(path.join(vault,'skills')),baseline.skills);
  await browser.saveScreenshot(path.join(evidence,'all-results-ar.png'));writeFileSync(path.join(evidence,'batch-result.json'),JSON.stringify({samples,records,fixture_provider:true,paid_calls:0,approved_description_and_dist_unchanged:true},null,2)+'\n');
  await $('[data-testid="cancel-classify"]').click();await $('[data-testid="onboarding-next"]').click();await $('[data-testid="onboarding-router"]').waitForDisplayed();await $('[data-testid="onboarding-finish"]').click();
 });
 it('remains available in settings; cancelling mid-batch saves the completed skill in one commit and stops the rest',async()=>{
  await $('[data-testid="nav-settings"]').click();await $('[data-testid="settings-tab-sources"]').click();await $('[data-testid="settings-classify-all"]').waitForEnabled();
  assert.equal(await $('[data-testid="settings-scan-paths"]').isDisplayed(),false);await $('[data-testid="manual-scan-toggle"]').click();await $('[data-testid="settings-scan-paths"]').waitForDisplayed();await $('[data-testid="manual-scan-toggle"]').click();
  const git=args=>spawnSync('git',['-C',vault,...args],{encoding:'utf8'}).stdout.trim();
  const before={dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills')),second:readFileSync(path.join(vault,'meta',ids[1]+'.toml'))};const commits=Number(git(['rev-list','--count','HEAD']));const count=trace().length;
  await $('[data-testid="settings-classify-all"]').click();await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').click();
  // Hold the second call: the first one is already past the hold check.
  await browser.waitUntil(()=>trace().length>count,{interval:10});writeFileSync(process.env.KINANA_CLASSIFY_HOLD,'hold');
  await browser.waitUntil(()=>trace().length>count+1,{timeout:20000});
  const progress=await invoke('classify_status');assert.equal(progress.done,false);assert.equal(progress.progress.total,2);assert.equal(progress.progress.completed,1);assert.equal(progress.progress.current_id,ids[1]);
  await browser.saveScreenshot(path.join(evidence,'all-progress-ar.png'));const pid=trace().at(-1).pid;
  await $('[data-testid="cancel-classify"]').click();
  await $('[data-testid="classification-stopped"]').waitForDisplayed({timeout:20000});
  assert.equal(await $('[data-testid="classification-stopped"]').getAttribute('data-reason'),'CANCELLED');
  assert.match(await $('[data-testid="classification-stopped-counts"]').getText(),/صُنّفت: [1١] · بقيت دون تصنيف: [1١]/);
  await browser.saveScreenshot(path.join(evidence,'all-cancelled-ar.png'));
  await browser.waitUntil(()=>{try{process.kill(pid,0);return false;}catch(e){if(e.code==='ESRCH')return true;throw e;}});unlinkSync(process.env.KINANA_CLASSIFY_HOLD);
  // Both calls happened; exactly one commit, holding only the completed skill.
  assert.equal(trace().length-count,2);
  assert.equal(Number(git(['rev-list','--count','HEAD'])),commits+1);
  assert.deepEqual(git(['show','--name-only','--format=','HEAD']).split('\n').filter(f=>f.startsWith('meta/')),[`meta/${ids[0]}.toml`]);
  assert.ok(cli(['show',ids[0]]).metadata.proposal.role.endsWith(` ${count+1}`));
  assert.deepEqual(readFileSync(path.join(vault,'meta',ids[1]+'.toml')),before.second);
  assert.deepEqual({dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills'))},{dist:before.dist,skills:before.skills});
  assert.equal(git(['status','--porcelain']),'');
  await $('[data-testid="cancel-classify"]').click();await browser.waitUntil(async()=>!await $('.classify-dialog').isExisting());
  writeFileSync(path.join(evidence,'cancel.json'),JSON.stringify({progress,saved:[ids[0]],remaining:[ids[1]],one_commit:true,calls:2,fixture_provider:true,paid_calls:0},null,2)+'\n');
 });
});
