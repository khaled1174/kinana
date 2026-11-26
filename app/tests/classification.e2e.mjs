import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,existsSync,readdirSync,lstatSync,unlinkSync,mkdirSync} from 'node:fs';
import {syntheticSkill} from './support/synthetic-skill.mjs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
const home=process.env.KINANA_UI_HOME,vault=path.join(home,'KinanaVault'),evidence=process.env.KINANA_UI_EVIDENCE;
const trace=()=>existsSync(process.env.KINANA_CLASSIFY_TRACE)?readFileSync(process.env.KINANA_CLASSIFY_TRACE,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
function cli(args){const p=spawnSync(process.env.KINANA_TEST_CLI,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home,PATH:process.env.KINANA_CLASSIFY_BIN+path.delimiter+process.env.PATH},encoding:'utf8'});assert.equal(p.status,0,p.stdout+p.stderr);return JSON.parse(p.stdout);}
function tree(root){const result={};function visit(at){for(const name of readdirSync(at).sort()){const p=path.join(at,name);if(lstatSync(p).isDirectory())visit(p);else result[path.relative(root,p)]=createHash('sha256').update(readFileSync(p)).digest('hex');}}visit(root);return result;}
async function invoke(command,params={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),command,params);}
const ids=['emil-color','emil-design-foundations'];
// r5/r6 (owner, 2026-11-24): classification approves the descriptions of
// skills that are not risky by default. The tests before r5 cover the manual
// review, so the vault starts with that off; the r6 test turns it on from the
// settings screen.
function manualReview(){const p=path.join(vault,'box.toml');writeFileSync(p,'auto_approve_descriptions = false\n'+readFileSync(p,'utf8'));const g=spawnSync('git',['-C',vault,'-c','user.name=Kinana Test','-c','user.email=test@example.invalid','commit','-qam','Review descriptions by hand'],{encoding:'utf8'});assert.equal(g.status,0,g.stderr);}
const git=args=>spawnSync('git',['-C',vault,...args],{encoding:'utf8'}).stdout.trim();
const piTrace=()=>existsSync(process.env.KINANA_PI_TRACE)?readFileSync(process.env.KINANA_PI_TRACE,'utf8').trim().split('\n').filter(Boolean).map(JSON.parse):[];
let baseline,approved,baseCalls;
describe('Classify every vault skill through native core',()=>{
 it('offers the action in first run, previews every skill, and cancels without provider calls',async()=>{
  await $('[data-testid="onboarding"]').waitForDisplayed();if(await $('html').getAttribute('lang')!=='ar')await $('[data-testid="language"]').click();
  await $('[data-testid="settings-initialize"]').click();await $('[data-testid="settings-preview"]').waitForDisplayed();await $('[data-testid="confirm-settings"]').click();await $('[data-testid="onboarding-next"]').waitForDisplayed();manualReview();await $('[data-testid="onboarding-next"]').click();await $('[data-testid="onboarding-skip-scan"]').click();
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
  await $('[data-testid="scan-agent-codex"]').click();await $('[data-testid="scan-agent-skills"]').click();await $('[data-testid="scan-results"]').waitForDisplayed();await $('[data-testid="confirm-scan-import"]').waitForEnabled();await $('[data-testid="confirm-scan-import"]').click();await browser.waitUntil(async()=>!await $('.scan-dialog').isExisting());
  writeFileSync(path.join(evidence,'automatic-discovery.json'),JSON.stringify({locations,real_skill_fixtures:true,manual_path_required:false,read_only_before_confirmation:true,missing_roots_explained:true,paid_calls:0},null,2)+'\n');
  await $('[data-testid="settings-classify-all"]').waitForEnabled();
  cli(['classify','emil-color','--agent','claude','--yes']);cli(['review','emil-color','--accept']);
  approved=cli(['show','emil-color']).metadata.description;assert.equal(approved.active,'improved');
  baseline={meta:tree(path.join(vault,'meta')),dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills'))};baseCalls=trace().length;
  await browser.saveScreenshot(path.join(evidence,'onboarding-all-ar.png'));
  const single=await invoke('preview_classify',{id:'emil-color'});assert.equal(single.selected,1);assert.equal(single.estimated_agent_calls,1);
  const preview=await invoke('preview_classify',{all:true});assert.equal(preview.selected,2);assert.equal(preview.estimated_agent_calls,2);assert.equal(preview.items.length,2);
  const pending=await invoke('preview_classify',{pending:true});
  assert.deepEqual(pending.items.map(item=>item.id),[ids[1]]);
  await $('[data-testid="settings-classify-pending"]').click();
  await $('[data-testid="classify-estimate"]').waitForDisplayed();
  assert.match(await $('[data-testid="classify-count"]').getText(),/1|١/);
  await $('[data-testid="cancel-classify"]').click();
  assert.equal(trace().length,baseCalls);
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
  for(const record of records)assert.equal(record.metadata.classification.classified_by,'claude');
  // r6 (owner, 2026-11-24): emil-color's approved description equals the new
  // proposal, so nothing waits for it; the other skill's proposal waits (this
  // vault reviews by hand).
  assert.equal(records[0].metadata.proposal,undefined);assert.ok(records[1].metadata.proposal);
  const states=await browser.execute(()=>Object.fromEntries([...document.querySelectorAll('[data-testid="classification-items"] li')].map(li=>[li.dataset.id,li.dataset.state])));
  assert.deepEqual(states,{[ids[0]]:'unchanged',[ids[1]]:'proposal'});
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
  // r6: the proposal matched the approved description, so the saved
  // classification carries this call's role and no proposal waits.
  assert.ok(cli(['show',ids[0]]).metadata.classification.role.endsWith(` ${count+1}`));assert.equal(cli(['show',ids[0]]).metadata.proposal,undefined);
  assert.deepEqual(readFileSync(path.join(vault,'meta',ids[1]+'.toml')),before.second);
  assert.deepEqual({dist:tree(path.join(vault,'dist')),skills:tree(path.join(vault,'skills'))},{dist:before.dist,skills:before.skills});
  assert.equal(git(['status','--porcelain']),'');
  await $('[data-testid="cancel-classify"]').click();await browser.waitUntil(async()=>!await $('.classify-dialog').isExisting());
  writeFileSync(path.join(evidence,'cancel.json'),JSON.stringify({progress,saved:[ids[0]],remaining:[ids[1]],one_commit:true,calls:2,fixture_provider:true,paid_calls:0},null,2)+'\n');
 });
 it('shows a structured refusal and confirms keywords for only that skill',async()=>{
  const refusal=path.join(process.env.KINANA_UI_BASE,'refuse-classification');
  writeFileSync(refusal,ids[1]);
  try {
   await $('[data-testid="settings-classify-all"]').click();
   await $('[data-testid="confirm-classify"]').waitForEnabled();
   await $('[data-testid="confirm-classify"]').click();
   const row=$('[data-testid="classification-skipped"] [data-reason="AGENT_REFUSED"]');
   await row.waitForDisplayed();
   assert.match(await row.getText(),/رفض Claude/);
   const savedSkip = await invoke('skill_detail',{id:ids[1]});
   assert.equal(savedSkip.skipped_classification.reason_code,'AGENT_REFUSED');
   assert.equal(savedSkip.skipped_classification.agent,'claude');
   const other=cli(['show',ids[0]]).metadata;
   const calls=trace().length;
   await $(`[data-testid="classify-skipped-keywords-${ids[1]}"]`).click();
   await $('[data-testid="confirm-classify"]').waitForEnabled();
   assert.equal(trace().length,calls);
   await $('[data-testid="confirm-classify"]').click();
   await $('[data-testid="classification-result"]').waitForDisplayed();
   assert.equal(cli(['show',ids[1]]).metadata.classification.classified_by,'keywords');
   assert.equal((await invoke('skill_detail',{id:ids[1]})).skipped_classification,undefined);
   assert.deepEqual(cli(['show',ids[0]]).metadata,other);
   assert.equal(trace().length,calls);
   await $('[data-testid="cancel-classify"]').click();
  } finally { unlinkSync(refusal); }
 });

 // r4: defects from the owner's test of the signed 0.3.0-beta-r3.
 const longIds=['reap-19-kernel-and-driver-exploitation-research','reap-20-advanced-software-cloning-and-feature-reconstruction'];
 const refusalFile=()=>path.join(process.env.KINANA_UI_BASE,'refuse-classification');
 const silentFile=()=>process.env.KINANA_CLASSIFY_SILENT;
 async function pick(id,value){
  const trigger=$(`[data-testid="${id}"]`);
  const option=$(`[data-testid="${id}-${value}"]`);
  await trigger.scrollIntoView({block:'center'});
  await trigger.click();
  await browser.waitUntil(async()=>await trigger.getAttribute('aria-expanded')==='true');
  await option.waitForExist();
  await browser.execute((id,value)=>{
   document.querySelector(`[data-testid="${id}-${value}"]`).focus({preventScroll:true});
  },id,value);
  await browser.keys('Enter');
  await browser.waitUntil(async()=>await trigger.getAttribute('aria-expanded')==='false');
 }
 async function sourcesTab(){await $('[data-testid="nav-vault"]').click();await $('[data-testid="nav-settings"]').click();await $('[data-testid="settings-tab-sources"]').waitForDisplayed();await $('[data-testid="settings-tab-sources"]').click();}
 async function closeDialog(){await $('[data-testid="cancel-classify"]').click();await browser.waitUntil(async()=>!await $('.classify-dialog').isExisting());}
 const alive=pid=>{try{process.kill(pid,0);return true;}catch(e){if(e.code==='ESRCH')return false;throw e;}};
 it('r4 item 1: saves the chosen Claude model and effort, shows them in the preview and passes them to Claude',async()=>{
  await sourcesTab();
  const status=$('[data-testid="settings-claude-status"]');await status.waitForDisplayed();
  assert.equal(await status.getAttribute('data-state'),'saved');
  const before=readFileSync(path.join(vault,'box.toml'),'utf8');assert.doesNotMatch(before,/classification_claude/);
  await pick('settings-claude-model','haiku');await pick('settings-claude-effort','high');
  // Not saved yet: the screen says so and classification waits for the save.
  assert.equal(await status.getAttribute('data-state'),'unsaved');
  assert.equal(await $('[data-testid="settings-classify-all"]').isEnabled(),false);
  assert.ok(await $('[data-testid="settings-classify-needs-save"]').isDisplayed());
  assert.equal(readFileSync(path.join(vault,'box.toml'),'utf8'),before);
  await status.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r4-claude-unsaved-ar.png'));
  await $('[data-testid="settings-claude-save"]').click();
  await $('[data-testid="settings-preview"]').waitForDisplayed();await $('[data-testid="confirm-settings"]').scrollIntoView();await $('[data-testid="confirm-settings"]').click();
  await browser.waitUntil(async()=>!await $('.settings-confirm-dialog').isExisting());
  await browser.waitUntil(async()=>await status.getAttribute('data-state')==='saved');
  const toml=readFileSync(path.join(vault,'box.toml'),'utf8');
  const section=toml.split('[classification_claude]')[1]?.split('\n[')[0]??'';
  assert.match(section,/^model = "haiku"$/m);assert.match(section,/^effort = "high"$/m);
  assert.match(await status.getText(),/haiku/);assert.match(await status.getText(),/high/);
  assert.equal(await $('[data-testid="settings-classify-all"]').isEnabled(),true);
  await status.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r4-claude-saved-ar.png'));
  const count=trace().length;
  await $('[data-testid="settings-classify-all"]').click();await $('[data-testid="classify-claude-model"]').waitForDisplayed();
  assert.match(await $('[data-testid="classify-claude-model"]').getText(),/haiku/);assert.match(await $('[data-testid="classify-estimate"]').getText(),/high/);
  await browser.saveScreenshot(path.join(evidence,'r4-claude-preview-ar.png'));
  await $('[data-testid="confirm-classify"]').click();await $('[data-testid="classification-result"]').waitForDisplayed({timeout:30000});
  const calls=trace().slice(count);assert.equal(calls.length,ids.length);
  for(const call of calls){assert.equal(call.args[call.args.indexOf('--model')+1],'haiku');assert.equal(call.args[call.args.indexOf('--effort')+1],'high');}
  await closeDialog();
  writeFileSync(path.join(evidence,'r4-claude-choice.json'),JSON.stringify({box_toml:'[classification_claude]'+section,preview_model:'haiku',preview_effort:'high',agent_arguments:calls.map(c=>c.args.slice(c.args.indexOf('--model'))),fixture_provider:true,paid_calls:0},null,2)+'\n');
 });
 it('r4 item 2: lists skipped skills with long names on one line and their reasons visible at 800×600, RTL and LTR',async()=>{
  const source=path.join(process.env.KINANA_UI_BASE,'r4-long-names');
  for(const id of longIds)syntheticSkill(path.join(source,id),id);
  cli(['add',source,'--all']);
  writeFileSync(refusalFile(),longIds.join('\n'));
  const results={};
  try{
   await browser.setWindowSize(800,600);
   for(const locale of ['ar','en']){
    if(await $('html').getAttribute('lang')!==locale)await $('[data-testid="language"]').click();
    await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
    await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
    await $('[data-testid="classification-skipped"]').waitForDisplayed({timeout:30000});
    await $('[data-testid="classification-skipped"]').scrollIntoView();
    const layout=await browser.execute(()=>{const list=document.querySelector('.skipped-items'),lr=list.getBoundingClientRect();return {viewport:[innerWidth,innerHeight],list:{scrollWidth:list.scrollWidth,clientWidth:list.clientWidth},rows:[...list.querySelectorAll('li')].map(li=>{const name=li.querySelector('[data-testid=classification-skipped-name]'),reason=li.querySelector('[data-testid=classification-skipped-reason]'),style=getComputedStyle(name),r=name.getBoundingClientRect(),rr=reason.getBoundingClientRect();return {title:name.title,text:name.textContent,whiteSpace:style.whiteSpace,fontSize:parseFloat(style.fontSize),height:r.height,width:r.width,truncated:name.scrollWidth>name.clientWidth,reason:reason.textContent.trim(),reasonBox:[rr.width,rr.height],reasonInside:rr.left>=lr.left-1&&rr.right<=lr.right+1,reasonBelowName:rr.top>=r.bottom-1,nameInside:r.left>=lr.left-1&&r.right<=lr.right+1,direction:getComputedStyle(li).direction};})};});
    // 800×600 is the window (the app's minimum size); the page is 800 wide below the title bar.
    const size=await browser.getWindowSize();assert.deepEqual([size.width,size.height],[800,600]);assert.equal(layout.viewport[0],800);layout.window=[size.width,size.height];
    assert.ok(layout.list.scrollWidth<=layout.list.clientWidth,JSON.stringify(layout.list));
    assert.deepEqual(layout.rows.map(r=>r.title).sort(),[...longIds].sort());
    for(const row of layout.rows){
     assert.equal(row.text,row.title);assert.equal(row.whiteSpace,'nowrap');
     assert.ok(row.height<=row.fontSize*2.2,JSON.stringify(row));assert.ok(row.width>row.fontSize*10,JSON.stringify(row));
     assert.ok(row.nameInside&&row.reasonInside&&row.reasonBelowName,JSON.stringify(row));
     assert.ok(row.reason.length>5&&row.reasonBox[0]>0&&row.reasonBox[1]>0,JSON.stringify(row));
     assert.equal(row.direction,locale==='ar'?'rtl':'ltr');
    }
    assert.match(layout.rows[0].reason,locale==='ar'?/رفض Claude/:/Claude/);
    results[locale]=layout;
    await browser.saveScreenshot(path.join(evidence,`r4-skipped-800x600-${locale}.png`));
    await closeDialog();
   }
  } finally { unlinkSync(refusalFile()); await browser.setWindowSize(1100,760); }
  if(await $('html').getAttribute('lang')!=='ar')await $('[data-testid="language"]').click();
  writeFileSync(path.join(evidence,'r4-skipped-layout.json'),JSON.stringify(results,null,2)+'\n');
 });
 it('r4 item 3: shows the time on the current skill and cancels an agent that never answers at once',async()=>{
  writeFileSync(silentFile(),'silent');const count=trace().length;let pid,latency,elapsed;
  try{
   await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
   await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
   await browser.waitUntil(()=>trace().length>count,{timeout:20000,interval:20});pid=trace().at(-1).pid;
   const clock=$('[data-testid="classify-elapsed"]');await clock.waitForDisplayed();
   const first=Number(await clock.getAttribute('data-seconds'));
   await browser.waitUntil(async()=>Number(await clock.getAttribute('data-seconds'))>=first+3,{timeout:8000});
   elapsed=await clock.getText();assert.match(elapsed,/[0-9٠-٩]:[0-9٠-٩]{2}/);assert.match(elapsed,/[2٢]:[0٠]{2}/);
   assert.equal((await invoke('classify_status')).done,false);
   await browser.saveScreenshot(path.join(evidence,'r4-elapsed-ar.png'));
   const started=Date.now();
   await $('[data-testid="cancel-classify"]').click();
   await $('[data-testid="classification-stopped"]').waitForDisplayed({timeout:20000});latency=Date.now()-started;
   assert.equal(await $('[data-testid="classification-stopped"]').getAttribute('data-reason'),'CANCELLED');
   assert.ok(latency<5000,`cancel took ${latency} ms`);
   assert.match(await $('[data-testid="classification-stopped-counts"]').getText(),/صُنّفت: [0٠] · بقيت دون تصنيف: [2٢]/);
   await browser.waitUntil(()=>!alive(pid),{timeout:5000});
   assert.equal(trace().length-count,1);
   await browser.saveScreenshot(path.join(evidence,'r4-cancel-silent-ar.png'));
  } finally { if(existsSync(silentFile()))unlinkSync(silentFile()); }
  await closeDialog();
  for(const id of longIds)assert.equal(cli(['show',id]).metadata.classification.classified_by,'pending');
  writeFileSync(path.join(evidence,'r4-cancel-silent.json'),JSON.stringify({agent_answered:false,elapsed_shown:elapsed,cancel_latency_ms:latency,agent_process_stopped:true,calls:1,fixture_provider:true,paid_calls:0},null,2)+'\n');
 }).timeout(90000);
 it('r4 item 3: skips an agent that never answers once the skill reaches its 120-second limit, then continues',async()=>{
  writeFileSync(silentFile(),'silent');const count=trace().length;const samples=[];let first,skippedAt;
  try{
   await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
   await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
   await browser.waitUntil(()=>trace().length>count,{timeout:20000,interval:20});first=trace().at(-1);
   await browser.waitUntil(async()=>{const s=await invoke('classify_status');samples.push({at:Date.now()/1000,progress:s.progress});if(s.progress.skipped>=1){skippedAt=Date.now()/1000;return true;}return false;},{timeout:150000,interval:250});
   unlinkSync(silentFile());
  } finally { if(existsSync(silentFile()))unlinkSync(silentFile()); }
  const waited=skippedAt-first.started;
  assert.ok(waited>=110&&waited<=130,`skipped after ${waited} s`);
  assert.equal(alive(first.pid),false);
  const row=$(`[data-testid="classification-skipped"] [data-reason="AGENT_TIMEOUT"]`);await row.waitForDisplayed({timeout:30000});
  assert.match(await row.getText(),/المهلة|time/i);
  const calls=trace().slice(count);
  // One attempt for the silent skill (its 120 s were spent), one for the next skill.
  assert.equal(calls.length,2);assert.equal(calls[0].pid,first.pid);
  assert.equal(await $$('[data-testid="classification-skipped"] li').length,1);
  await $('[data-testid="classification-skipped"]').scrollIntoView();await browser.saveScreenshot(path.join(evidence,'r4-timeout-skipped-ar.png'));
  const skippedId=await row.$('[data-testid="classification-skipped-name"]').getAttribute('title');
  await closeDialog();
  assert.equal(cli(['show',skippedId]).metadata.classification.classified_by,'pending');
  const other=longIds.find(id=>id!==skippedId);assert.equal(cli(['show',other]).metadata.classification.classified_by,'claude');
  writeFileSync(path.join(evidence,'r4-timeout.json'),JSON.stringify({agent_answered:false,seconds_until_skip:waited,reason_code:'AGENT_TIMEOUT',skipped:skippedId,continued_with:other,calls:calls.length,samples:samples.filter((_,i)=>i%20===0),fixture_provider:true,paid_calls:0},null,2)+'\n');
 }).timeout(240000);
 it('r4 item 4: shows each audit reason with its file, line and excerpt under the audit line',async()=>{
  const id='r4-risky-cleanup',source=path.join(process.env.KINANA_UI_BASE,'r4-risky'),folder=path.join(source,id);
  syntheticSkill(folder,id);mkdirSync(path.join(folder,'scripts'),{recursive:true});
  // The owner's example in r3: the excerpt of a plain `rm -rf "$NAME"` is shown.
  writeFileSync(path.join(folder,'scripts/cleanup.sh'),'#!/bin/sh\nset -eu\nrm -rf "$TEST_ROOT"\n');
  cli(['add',source,'--all','--allow-risky']);
  const audit=cli(['show',id]).metadata.audit;assert.equal(audit.level,'risky');
  const detail=audit.details.find(d=>d.level==='risky');assert.equal(detail.path,'scripts/cleanup.sh');assert.equal(detail.line,3);
  const shown={};
  for(const locale of ['ar','en']){
   if(await $('html').getAttribute('lang')!==locale)await $('[data-testid="language"]').click();
   await $('[data-testid="nav-vault"]').click();await $(`[data-testid="skill-${id}"] .skill-select`).waitForDisplayed();await $(`[data-testid="skill-${id}"] .skill-select`).click();
   const reasons=$('[data-testid="detail-audit-reasons"]');await reasons.waitForDisplayed();
   // No hidden step: open by default for a risky skill, on the first tab.
   assert.equal(await browser.execute(()=>document.querySelector('[data-testid="detail-audit-reasons"]').open),true);
   assert.equal(await $('[data-testid="tab-preview"]').getAttribute('aria-selected'),'true');
   const text=await reasons.getText();
   assert.equal(await reasons.$('[data-testid="audit-location"]').getText(),'scripts/cleanup.sh:3');assert.match(text,/rm -rf "\$TEST_ROOT"/);
   const quote=reasons.$('blockquote');assert.equal(await quote.isDisplayed(),true);
   // The visual order of the location in this locale, read from the rendered boxes.
   const order=await browser.execute(()=>{const el=document.querySelector('[data-testid="detail-audit-reasons"] [data-testid="audit-location"]');const path=el.querySelector('.path-view').getBoundingClientRect();const box=el.getBoundingClientRect();return {pathLeft:path.left,boxLeft:box.left,boxRight:box.right,pathRight:path.right};});
   assert.ok(order.pathLeft<=order.boxLeft+1&&order.pathRight<order.boxRight,JSON.stringify(order));
   shown[locale]=text;
   await browser.saveScreenshot(path.join(evidence,`r4-audit-reasons-${locale}.png`));
  }
  if(await $('html').getAttribute('lang')!=='ar')await $('[data-testid="language"]').click();
  writeFileSync(path.join(evidence,'r4-audit-reasons.json'),JSON.stringify({skill:id,level:audit.level,detail,shown},null,2)+'\n');
 });

 // r5: the owner's notes on the signed 0.3.0-beta-r4.
 const LIST_ARGUMENTS=['--no-tools','--no-mcp','--no-extensions','--no-skills','--no-context-files','--no-prompt-templates','--no-themes','--no-session','--no-approve','--offline','--list-models'];
 const hiddenValue='Zx81kQ0pLm4Rt7Vb2Nc6Hy9Wd3Fg5Js1';
 it('r5 item 4: groups audit reasons by rule and file, risky first, with the variable names and forms and never a value',async()=>{
  const id='r5-env-reads',source=path.join(process.env.KINANA_UI_BASE,'r5-env'),folder=path.join(source,id);
  syntheticSkill(folder,id);mkdirSync(path.join(folder,'scripts'),{recursive:true});mkdirSync(path.join(folder,'tools'),{recursive:true});
  writeFileSync(path.join(folder,'scripts/env.sh'),`#!/bin/sh\necho "$GITHUB_TOKEN"\nprintenv\nAPI_KEY="${hiddenValue}"\necho "\${1:-default}"\n`);
  writeFileSync(path.join(folder,'tools/read.py'),'import os\nname = input()\nvalue = os.environ.get(name)\n');
  cli(['add',source,'--all','--allow-risky']);
  const audit=cli(['show',id]).metadata.audit;assert.equal(audit.level,'risky');
  // Contract 11 (owner, 2026-11-24, r7): --json carries the forms of secrets findings only.
  assert.ok(audit.details.some(d=>d.rule==='secrets'&&d.path==='scripts/env.sh'&&d.forms?.includes('$GITHUB_TOKEN')),JSON.stringify(audit.details));
  assert.ok(audit.details.every(d=>d.rule==='secrets'||!('forms' in d)),JSON.stringify(audit.details));
  assert.ok(!JSON.stringify(audit).includes(hiddenValue));
  const shown={};
  for(const locale of ['ar','en']){
   if(await $('html').getAttribute('lang')!==locale)await $('[data-testid="language"]').click();
   // Through Settings, so the vault list is read again after the CLI added the skill.
   await $('[data-testid="nav-settings"]').click();await $('[data-testid="nav-vault"]').click();await $(`[data-testid="skill-${id}"] .skill-select`).waitForDisplayed();await $(`[data-testid="skill-${id}"] .skill-select`).click();
   const reasons=$('[data-testid="detail-audit-reasons"]');await reasons.waitForDisplayed();
   const groups=await browser.execute(()=>[...document.querySelectorAll('[data-testid="detail-audit-reasons"] [data-testid="audit-group"]')].map(g=>({rule:g.dataset.rule,level:g.dataset.level,open:g.open,single:g.dataset.single==='true',heading:!!g.querySelector('summary'),locations:g.querySelectorAll('[data-testid="audit-location"]').length,count:Number(g.dataset.count),summary:(g.querySelector('summary')??g).innerText})));
   // Risky groups first; one group per rule and file.
   const firstReview=groups.findIndex(g=>g.level!=='risky');assert.ok(firstReview>0&&groups.slice(firstReview).every(g=>g.level!=='risky'),JSON.stringify(groups));
   const shell=groups.find(g=>g.rule==='secrets'&&g.summary.includes('scripts/env.sh'));assert.ok(shell,JSON.stringify(groups));
   assert.equal(shell.count,4);assert.equal(shell.level,'risky');assert.equal(shell.open,false);
   assert.match(shell.summary,locale==='ar'?/أسرار أو وصول لمتغيرات بيئة · [4٤] مواضع/:/Secrets or environment variable access · 4 places/);
   assert.match(shell.summary,/\$GITHUB_TOKEN/);
   assert.equal(shell.single,false);assert.equal(shell.heading,true);
   // r6 (owner): one place is one line, without a group heading repeating it.
   const py=groups.find(g=>g.rule==='secrets'&&g.summary.includes('tools/read.py'));assert.ok(py,JSON.stringify(groups));
   assert.equal(py.count,1);assert.equal(py.single,true);assert.equal(py.heading,false);assert.equal(py.locations,1);
   assert.equal(py.summary.split('tools/read.py').length-1,1,py.summary);assert.match(py.summary,/tools\/read\.py:3/);
   assert.doesNotMatch(py.summary,locale==='ar'?/موضع/:/\bplaces?\b/);
   // Expand the shell group to its lines.
   await browser.execute(()=>{for(const g of document.querySelectorAll('[data-testid="detail-audit-reasons"] [data-testid="audit-group"][data-rule="secrets"]'))g.open=true;});
   const text=await reasons.getText();
   for(const form of ['$GITHUB_TOKEN','printenv','API_KEY = …','${1}','os.environ.get(…)'])assert.ok(text.includes(form),form+'\n'+text);
   assert.ok(text.includes(locale==='ar'?'قيمة سرية حرفية (محجوبة)':'literal secret value (hidden)'));
   assert.ok(!(await $('body').getText()).includes(hiddenValue),'a literal value reached the screen');
   assert.equal(await reasons.$('[data-testid="audit-location"]').getText(),'scripts/env.sh:2');
   shown[locale]={groups,text};
   await reasons.scrollIntoView({block:'start'});await browser.saveScreenshot(path.join(evidence,`r5-audit-groups-${locale}.png`));
  }
  if(await $('html').getAttribute('lang')!=='ar')await $('[data-testid="language"]').click();
  writeFileSync(path.join(evidence,'r5-audit-groups.json'),JSON.stringify({skill:id,level:audit.level,shown,literal_value_on_screen:false},null,2)+'\n');
 });
 it('r5 item 3: says every skill is classified instead of hiding the classify-pending button',async()=>{
  await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
  await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
  await $('[data-testid="classification-result"]').waitForDisplayed({timeout:60000});await closeDialog();
  const pending=cli(['list']).map(s=>s.id).filter(id=>cli(['show',id]).metadata.classification.classified_by==='pending');assert.deepEqual(pending,[]);
  const all=$('[data-testid="settings-all-classified"]');await all.waitForDisplayed();
  assert.equal(await all.getText(),'كل المهارات مصنّفة ✓');assert.equal(await $('[data-testid="settings-classify-pending"]').isExisting(),false);
  await all.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r5-all-classified-settings-ar.png'));
  await $('[data-testid="nav-vault"]').click();const vaultAll=$('[data-testid="vault-all-classified"]');await vaultAll.waitForDisplayed();
  assert.equal(await vaultAll.getText(),'كل المهارات مصنّفة ✓');assert.equal(await $('[data-testid="vault-classify-pending"]').isExisting(),false);
  await browser.saveScreenshot(path.join(evidence,'r5-all-classified-vault-ar.png'));
  await $('[data-testid="language"]').click();assert.equal(await $('[data-testid="vault-all-classified"]').getText(),'All skills are classified ✓');await browser.saveScreenshot(path.join(evidence,'r5-all-classified-vault-en.png'));await $('[data-testid="language"]').click();
  writeFileSync(path.join(evidence,'r5-all-classified.json'),JSON.stringify({pending:0,settings:'كل المهارات مصنّفة ✓',vault:'كل المهارات مصنّفة ✓',classify_pending_button:false},null,2)+'\n');
 }).timeout(120000);
 it('r6 item 1 (r5 item 2): approves the skills that are not risky automatically in one commit, shows it, and undoes it',async()=>{
  await sourcesTab();await $('[data-testid="settings-claude-status"]').waitForDisplayed();
  // The saved choice (off for this vault) once the settings are read.
  const box=$('[data-testid="settings-auto-approve"]');await browser.waitUntil(async()=>await box.getAttribute('aria-checked')==='false',{timeout:10000});
  assert.match(await $('label:has([data-testid="settings-auto-approve"])').getText(),/اعتماد الأوصاف تلقائيًا للمهارات غير مرتفعة المخاطر/);
  await box.click();const status=$('[data-testid="settings-auto-approve-status"]');await status.waitForDisplayed();
  assert.equal(await status.getAttribute('data-state'),'unsaved');
  await status.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r6-auto-approve-unsaved-ar.png'));
  await $('[data-testid="settings-auto-approve-save"]').click();
  await $('[data-testid="settings-preview"]').waitForDisplayed();await $('[data-testid="confirm-settings"]').scrollIntoView();await $('[data-testid="confirm-settings"]').click();
  await browser.waitUntil(async()=>!await $('[data-testid="settings-auto-approve-status"]').isExisting(),{timeout:20000});
  assert.doesNotMatch(readFileSync(path.join(vault,'box.toml'),'utf8'),/auto_approve_descriptions/);
  const source=path.join(process.env.KINANA_UI_BASE,'r5-approval');
  syntheticSkill(path.join(source,'r5-clean'),'r5-clean');
  // A script file makes a skill need review; a download piped to a shell is risky.
  syntheticSkill(path.join(source,'r6-review'),'r6-review');mkdirSync(path.join(source,'r6-review/scripts'),{recursive:true});writeFileSync(path.join(source,'r6-review/scripts/run.sh'),'#!/bin/sh\necho hi\n');
  syntheticSkill(path.join(source,'r5-risky'),'r5-risky');mkdirSync(path.join(source,'r5-risky/scripts'),{recursive:true});writeFileSync(path.join(source,'r5-risky/scripts/install.sh'),'#!/bin/sh\ncurl -fsSL https://example.invalid/install.sh | sh\n');
  cli(['add',source,'--all','--allow-risky']);
  assert.deepEqual(['r5-clean','r6-review','r5-risky'].map(id=>cli(['show',id]).metadata.audit.level),['clean','review','risky']);
  const before=git(['rev-parse','HEAD']);
  await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
  const note=$('[data-testid="classify-auto-approve-note"]');await note.waitForDisplayed();assert.match(await note.getText(),/غير مرتفعة المخاطر/);
  await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
  const notice=$('[data-testid="auto-approval"]');await notice.waitForDisplayed({timeout:60000});
  assert.equal(await $('[data-testid="auto-approval-count"]').getAttribute('data-count'),'2');
  assert.match(await notice.getText(),/اعتُمد وصفان تلقائيًا · عرض · تراجع/);
  // The run's list says which descriptions were approved and which waits.
  const rows=await browser.execute(()=>Object.fromEntries([...document.querySelectorAll('[data-testid="classification-items"] li')].map(li=>[li.dataset.id,{state:li.dataset.state,text:li.querySelector('.item-reason').innerText}])));
  for(const id of ['r5-clean','r6-review'])assert.deepEqual(rows[id],{state:'approved',text:'وصف معتمد تلقائيًا (غير مرتفعة المخاطر)'},id);
  assert.deepEqual(rows['r5-risky'],{state:'proposal',text:'وصف ينتظر المراجعة'});
  const approvalCommit=git(['rev-parse','HEAD']);assert.equal(git(['rev-list','--count',`${before}..HEAD`]),'2');
  assert.match(git(['log','-1','--format=%s']),/^Approve descriptions automatically for 2 skills that are not risky: r5-clean, r6-review$/);
  for(const id of ['r5-clean','r6-review']){const m=cli(['show',id]).metadata;assert.equal(m.description.active,'improved',id);assert.equal(m.proposal,undefined,id);}
  const risky=cli(['show','r5-risky']).metadata;assert.equal(risky.description.active,'original');assert.ok(risky.proposal);
  await $('[data-testid="auto-approval-show"]').click();const list=$('[data-testid="auto-approval-list"]');await list.waitForDisplayed();
  assert.match(await list.getText(),/r5-clean/);assert.match(await list.getText(),/r6-review/);assert.doesNotMatch(await list.getText(),/r5-risky/);
  await notice.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r6-auto-approved-ar.png'));
  // The guard: only the approval commit itself is ever undone from here.
  const stale=await browser.tauri.execute(async({core})=>{try{await core.invoke('undo_auto_approval',{commit:'0000000000000000000000000000000000000000'});return null;}catch(e){return e.code??JSON.stringify(e);}});
  assert.equal(stale,'AUTO_APPROVAL_NOT_LATEST');assert.equal(git(['rev-parse','HEAD']),approvalCommit);
  await $('[data-testid="auto-approval-undo"]').click();
  assert.match(await $('[data-testid="auto-approval-confirmation"]').getText(), /[2٢]/);
  await $('[data-testid="cancel-auto-approval-undo"]').click();
  assert.equal(git(['rev-parse','HEAD']),approvalCommit);
  await $('[data-testid="auto-approval-undo"]').click();
  await $('[data-testid="confirm-auto-approval-undo"]').click();
  const undone = $('[data-testid="auto-approval-undone"]');
  await undone.waitForDisplayed({timeout:20000});
  assert.match(await undone.getText(), /تم التراجع عن الاعتماد التلقائي/);
  assert.equal(git(['rev-parse','HEAD~1']),approvalCommit);assert.match(git(['log','-1','--format=%s']),/^Revert "Approve descriptions automatically/);
  for(const id of ['r5-clean','r6-review']){const m=cli(['show',id]).metadata;assert.equal(m.description.active,'original',id);assert.ok(m.proposal,id);assert.equal(m.classification.classified_by,'claude');}
  await browser.saveScreenshot(path.join(evidence,'r6-auto-approval-undone-ar.png'));
  await closeDialog();
  await $('[data-testid="language"]').click();await sourcesTab();const label=$('label:has([data-testid="settings-auto-approve"])');assert.match(await label.getText(),/Approve descriptions automatically for skills that are not high risk/);await label.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r6-auto-approve-setting-en.png'));await $('[data-testid="language"]').click();
  writeFileSync(path.join(evidence,'r6-auto-approval.json'),JSON.stringify({approved:['r5-clean','r6-review'],levels:{'r5-clean':'clean','r6-review':'review','r5-risky':'risky'},waiting:['r5-risky'],approval_commit:approvalCommit,commits_after_run:2,stale_undo:'AUTO_APPROVAL_NOT_LATEST',undone:true,fixture_provider:true,paid_calls:0},null,2)+'\n');
 }).timeout(120000);
 it('r6 item 2: reclassifying approved skills keeps them approved when the description matches, and a different one follows the same rule',async()=>{
  const override=path.join(process.env.KINANA_UI_BASE,'classification-description');
  const fixed='Review interface spacing, typography, color, and hierarchy before changing a screen. Use for interface design reviews; skip backend database work.';
  const other='Check interface spacing, color contrast and hierarchy before a screen changes; skip backend database work.';
  const three=['r5-clean','r6-review','r5-risky'];
  assert.doesNotMatch(readFileSync(path.join(vault,'box.toml'),'utf8'),/auto_approve_descriptions/);
  const rowsOf=()=>browser.execute(()=>Object.fromEntries([...document.querySelectorAll('[data-testid="classification-items"] li')].map(li=>[li.dataset.id,{state:li.dataset.state,text:li.querySelector('.item-reason').innerText}])));
  async function classifyAll(shot){
   await sourcesTab();await $('[data-testid="settings-classify-all"]').waitForEnabled();await $('[data-testid="settings-classify-all"]').click();
   await $('[data-testid="classify-estimate"]').waitForDisplayed();await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
   await $('[data-testid="classification-result"]').waitForDisplayed({timeout:150000});
   const rows=await rowsOf();await $('[data-testid="classification-result"]').scrollIntoView({block:'start'});await browser.saveScreenshot(path.join(evidence,shot));
   await closeDialog();return rows;
  }
  let shown;
  try{
   // The two that are not risky are approved automatically; the risky one by hand.
   const report=cli(['classify',...three,'--yes']);
   // Contract 11 (owner, 2026-11-24, r7): a field, not a warning.
   assert.deepEqual(report.auto_approved?.ids,['r5-clean','r6-review'],JSON.stringify(report));
   assert.ok(!report.warnings.some(w=>w.startsWith('AUTO_APPROVED')),JSON.stringify(report.warnings));
   cli(['review','r5-risky','--accept']);
   const approvedAt=Object.fromEntries(three.map(id=>{const m=cli(['show',id]).metadata;assert.equal(m.description.active,'improved',id);assert.equal(m.description.improved,fixed,id);assert.equal(m.proposal,undefined,id);return [id,m.description.approved_at];}));
   // The same description with other spacing: nothing waits, nothing changes.
   writeFileSync(override,'  '+fixed.replaceAll(', ',',   ')+'  ');
   const same=await classifyAll('r6-reclassified-same-ar.png');
   for(const id of three){
    assert.deepEqual(same[id],{state:'unchanged',text:'اكتمل؛ الوصف الحالي لم يتغير'},id);
    const m=cli(['show',id]).metadata;assert.equal(m.proposal,undefined,id);assert.equal(m.description.active,'improved',id);assert.equal(m.description.improved,fixed,id);assert.equal(m.description.approved_at,approvedAt[id],id);
   }
   // The skills list says "approved description", not "awaiting review".
   await $('[data-testid="nav-vault"]').click();
   for(const id of three){const row=$(`[data-testid="skill-${id}"]`);await row.waitForDisplayed();const text=await row.getText();assert.doesNotMatch(text,/وصف ينتظر المراجعة/,id);}
   await browser.saveScreenshot(path.join(evidence,'r6-reclassified-same-vault-ar.png'));
   // Another description: approved automatically unless the skill is risky.
   writeFileSync(override,other);
   const different=await classifyAll('r6-reclassified-different-ar.png');
   for(const id of ['r5-clean','r6-review']){
    assert.deepEqual(different[id],{state:'approved',text:'وصف معتمد تلقائيًا (غير مرتفعة المخاطر)'},id);
    const m=cli(['show',id]).metadata;assert.equal(m.proposal,undefined,id);assert.equal(m.description.improved,other,id);assert.notEqual(m.description.approved_at,approvedAt[id],id);
   }
   assert.deepEqual(different['r5-risky'],{state:'proposal',text:'وصف ينتظر المراجعة'});
   const risky=cli(['show','r5-risky']).metadata;assert.equal(risky.description.active,'improved');assert.equal(risky.description.improved,fixed);assert.equal(risky.proposal.improved_description,other);
   shown={same,different};
  } finally { if(existsSync(override))unlinkSync(override); }
  writeFileSync(path.join(evidence,'r6-reclassify-approved.json'),JSON.stringify({same_description_after_whitespace:Object.fromEntries(three.map(id=>[id,shown.same[id]])),different_description:Object.fromEntries(three.map(id=>[id,shown.different[id]])),fixture_provider:true,paid_calls:0},null,2)+'\n');
 }).timeout(400000);
 it('r5 item 1: lists Pi providers and models from Pi itself, offers "Pi default", and falls back to text fields with the reason',async()=>{
  await sourcesTab();await $('[data-testid="settings-claude-status"]').waitForDisplayed();const enabled=$('[data-testid="pi-enabled"]');await enabled.scrollIntoView({block:'center'});
  const traced=piTrace().length;
  await enabled.click();const status=$('[data-testid="pi-models-status"]');
  await browser.waitUntil(async()=>await status.getAttribute('data-state')==='ok',{timeout:30000});
  const calls=piTrace().slice(traced);assert.deepEqual(calls.map(c=>c.kind),['version','list']);
  const list=calls[1];assert.deepEqual(list.args,LIST_ARGUMENTS);assert.equal(list.stdin,'');assert.equal(list.offline,'1');assert.deepEqual(list.cwd_entries,[]);
  assert.match(await status.getText(),/1\.5\.2/);
  assert.match(await $('[data-testid="pi-settings"] .settings-help').getText(),/1\.5\.2 أو 1\.6\.1/);
  await pick('pi-provider-choice','openai-codex');
  assert.match(await $('[data-testid="pi-model-choice"]').getText(),/افتراضي Pi \(gpt-6\.1-sol\)/);
  await $('[data-testid="pi-model-choice"]').click();await $('[data-testid="pi-model-choice-gpt-5.5"]').waitForDisplayed();
  const options=await browser.execute(()=>[...document.querySelectorAll('[role=option]')].map(o=>o.innerText));
  await browser.keys('Escape');await browser.waitUntil(async()=>!await $('[role=listbox]').isExisting());
  assert.deepEqual(options,['افتراضي Pi (gpt-6.1-sol)','gpt-5.5','gpt-6.1-sol']);
  await pick('pi-provider-choice','local-lab');await $('[data-testid="pi-no-default"]').waitForDisplayed();
  assert.match(await $('[data-testid="pi-model-choice"]').getText(),/lab-model-a/);
  await pick('pi-provider-choice','openai-codex');assert.match(await $('[data-testid="pi-model-choice"]').getText(),/افتراضي Pi/);
  await status.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r5-pi-list-ar.png'));
  // The language changes in place: the unsaved Pi choice stays on screen.
  await $('[data-testid="language"]').click();await $('[data-testid="pi-models-status"]').scrollIntoView({block:'center'});
  assert.match(await $('[data-testid="pi-settings"] .settings-help').getText(),/1\.5\.2 or 1\.6\.1/);
  await browser.waitUntil(async()=>/Pi default \(gpt-6\.1-sol\)/.test(await $('[data-testid="pi-model-choice"]').getText()),{timeout:5000});await browser.saveScreenshot(path.join(evidence,'r5-pi-list-en.png'));
  await $('[data-testid="language"]').click();
  await pick('settings-agent','auto');
  await $('[data-testid="settings-save"]').scrollIntoView({block:'center'});await $('[data-testid="settings-save"]').click();
  await $('[data-testid="settings-preview"]').waitForDisplayed();await $('[data-testid="confirm-settings"]').scrollIntoView();await $('[data-testid="confirm-settings"]').click();
  await browser.waitUntil(()=>/\[classification_pi\]/.test(readFileSync(path.join(vault,'box.toml'),'utf8')),{timeout:20000});
  const toml=readFileSync(path.join(vault,'box.toml'),'utf8');const section=toml.split('[classification_pi]')[1].split('\n[')[0];
  assert.match(section,/^enabled = true$/m);assert.match(section,/^provider = "openai-codex"$/m);assert.match(section,/^model = ""$/m);assert.match(toml,/^classification_agent = "auto"$/m);
  // Failures: a clear reason and the text fields; Pi's own text never appears.
  const failures={};
  for(const [mode,state,code] of [['fail','error','AGENT_EXIT_FAILED'],['garbage','error','PI_MODELS_UNREADABLE'],['empty','empty',undefined]]){
   writeFileSync(process.env.KINANA_PI_LIST_MODE,mode);
   await sourcesTab();await $('[data-testid="pi-models-reread"]').scrollIntoView({block:'center'});await $('[data-testid="pi-models-reread"]').click();
   await browser.waitUntil(async()=>await $('[data-testid="pi-models-status"]').getAttribute('data-state')===state,{timeout:30000});
   if(code)assert.equal(await $('[data-testid="pi-models-status"]').getAttribute('data-code'),code);
   assert.equal(await $('[data-testid="pi-provider"]').getValue(),'openai-codex');assert.equal(await $('[data-testid="pi-model"]').getValue(),'');
   const body=await $('body').getText();assert.ok(!body.includes('private-raw-pi-error')&&!body.includes('refresh_token'),'raw Pi text reached the screen');
   failures[mode]={state,code,text:await $('[data-testid="pi-models-status"]').getText()};
   await $('[data-testid="pi-models-status"]').scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,`r5-pi-list-${mode}-ar.png`));
  }
  unlinkSync(process.env.KINANA_PI_LIST_MODE);
  writeFileSync(path.join(evidence,'r5-pi-list.json'),JSON.stringify({list_call:list,options,saved:'[classification_pi]'+section,failures,text_sent:false,paid_calls:0},null,2)+'\n');
 }).timeout(180000);
 it('r5 item 1: the Pi confirmation names "Pi default" and its model, and Pi receives exactly that provider and model',async()=>{
  const source=path.join(process.env.KINANA_UI_BASE,'r5-pi');syntheticSkill(path.join(source,'r5-pi'),'r5-pi');cli(['add',source,'--all']);
  writeFileSync(process.env.KINANA_CLASSIFY_BROKEN,'broken');const traced=piTrace().length;let runs;
  try{
   await sourcesTab();await $('[data-testid="settings-classify-pending"]').waitForEnabled();await $('[data-testid="settings-classify-pending"]').click();
   const preview=$('[data-testid="pi-preview-model"]');await preview.waitForDisplayed();
   assert.match(await preview.getText(),/openai-codex · gpt-6\.1-sol \(افتراضي Pi لهذا المزوّد\)/);
   await $('[data-testid="confirm-classify"]').waitForEnabled();await $('[data-testid="confirm-classify"]').click();
   const ask=$('[data-testid="pi-confirmation"]');await ask.waitForDisplayed({timeout:60000});
   const model=$('[data-testid="pi-confirmation-model"]');assert.equal(await model.getAttribute('data-default'),'true');
   assert.match(await model.getText(),/gpt-6\.1-sol · افتراضي Pi لهذا المزوّد/);assert.match(await ask.getText(),/openai-codex/);
   assert.equal(piTrace().slice(traced).filter(c=>c.kind==='run').length,0,'Pi was started before the approval');
   await ask.scrollIntoView({block:'center'});await browser.saveScreenshot(path.join(evidence,'r5-pi-confirmation-ar.png'));
   // Approving Pi returns to the preview of the remaining skills; the run starts on its confirmation.
   await $('[data-testid="pi-confirm"]').click();
   await $('[data-testid="confirm-classify"]').waitForEnabled({timeout:30000});
   assert.equal(piTrace().slice(traced).filter(c=>c.kind==='run').length,0);
   await $('[data-testid="confirm-classify"]').click();
   await $('[data-testid="classification-result"]').waitForDisplayed({timeout:60000});
   runs=piTrace().slice(traced).filter(c=>c.kind==='run');assert.equal(runs.length,1);
   const args=runs[0].args;assert.equal(args[args.indexOf('--provider')+1],'openai-codex');assert.equal(args[args.indexOf('--model')+1],'gpt-6.1-sol');
   for(const flag of ['-p','--no-tools','--offline','--no-session'])assert.ok(args.includes(flag),flag);
   assert.equal(cli(['show','r5-pi']).metadata.classification.classified_by,'pi');
   await browser.saveScreenshot(path.join(evidence,'r5-pi-classified-ar.png'));
  } finally { if(existsSync(process.env.KINANA_CLASSIFY_BROKEN))unlinkSync(process.env.KINANA_CLASSIFY_BROKEN); }
  await closeDialog();
  writeFileSync(path.join(evidence,'r5-pi-confirmation.json'),JSON.stringify({preview:'openai-codex · gpt-6.1-sol (Pi default)',confirmation_default:true,pi_arguments:runs.map(r=>r.args),fixture_provider:true,paid_calls:0},null,2)+'\n');
 }).timeout(180000);
 it('offers a fresh Claude estimate when Pi is pending review and waits for confirmation',async()=>{
  const pkg=path.join(process.env.KINANA_FAKE_PI_DIR,'package','package.json');
  const original=readFileSync(pkg,'utf8');
  const config=path.join(vault,'box.toml');
  const box=readFileSync(config,'utf8');
  assert.match(box,/classification_agent = "auto"/);
  writeFileSync(config,box.replace('classification_agent = "auto"','classification_agent = "pi"'));
  git(['add','--','box.toml']);git(['commit','-m','Select the synthetic Pi fixture']);
  const source=path.join(process.env.KINANA_UI_BASE,'c16-pending');
  syntheticSkill(path.join(source,'c16-pending'),'c16-pending');
  cli(['add',source,'--all']);
  writeFileSync(pkg,original.replace('1.5.2','1.7.0'));
  try {
   const before = {claude:trace().length,pi:piTrace().length};
   await sourcesTab();
   await $('[data-testid="settings-classify-pending"]').click();
   await $('[data-testid="confirm-classify"]').waitForEnabled();
   await $('[data-testid="confirm-classify"]').click();
   const version=$('[data-testid="pi-unreviewed-version"]');
   await version.waitForDisplayed();
   assert.match(await version.getText(),/1\.7\.0 قيد المراجعة؛ الإصدار المقبول 1\.5\.2, 1\.6\.1/);
   assert.equal(trace().length,before.claude);
   assert.equal(piTrace().length,before.pi);
   const claude=$('[data-testid="classify-with-claude"]');
   assert.match(await claude.getText(),/استخدم Claude لهذه الدفعة/);
   await claude.click();
   await $('[data-testid="classify-claude-model"]').waitForDisplayed();
   assert.equal(trace().length,before.claude);
   await $('[data-testid="confirm-classify"]').waitForEnabled();
   await $('[data-testid="confirm-classify"]').click();
   await $('[data-testid="classification-result"]').waitForDisplayed();
   assert.equal(cli(['show','c16-pending']).metadata.classification.classified_by,'claude');
   assert.equal(piTrace().length,before.pi);
   await closeDialog();
  } finally { writeFileSync(pkg,original); }
 }).timeout(60000);
});
