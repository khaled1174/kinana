import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,readdirSync,lstatSync,mkdirSync,cpSync,readlinkSync,existsSync,unlinkSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=fileURLToPath(new URL('../../',import.meta.url)),evidence=process.env.KINANA_UI_EVIDENCE||path.join(root,'docs/evidence/phase-6/batch-4-5');
const home=process.env.KINANA_UI_HOME,base=process.env.KINANA_UI_BASE,vault=path.join(home,'SkillsBox');
const project=path.join(base,'واجهة-تجربة');
function cli(args){const r=spawnSync(process.env.KINANA_TEST_CLI,['--vault',vault,'--json',...args],{env:{...process.env,HOME:home},encoding:'utf8'});assert.equal(r.status,0,r.stdout+r.stderr);return JSON.parse(r.stdout);}
function tree(root){return readdirSync(root).sort().flatMap(name=>{const p=path.join(root,name),m=lstatSync(p);return m.isSymbolicLink()?[[name,{link:readlinkSync(p)}]]:m.isDirectory()?[[name,{directory:tree(p)}]]:[[name,{sha256:createHash('sha256').update(readFileSync(p)).digest('hex')}]];});}
async function invoke(cmd,args={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),cmd,args);}
async function choose(testId, label) {
  await $(`[data-testid="${testId}"]`).click();
  const option = $(`//*[@role="option" and normalize-space(.)=${JSON.stringify(label)}]`);
  await option.waitForDisplayed();
  await option.click();
}
async function selectProject(p){await $('[data-testid="nav-projects"]').click();await $('[data-testid="refresh-projects"]').waitForDisplayed();await $('[data-testid="refresh-projects"]').click();const row=$(`[data-testid="project-row"][data-path="${p}"]`);await row.waitForDisplayed();await row.click();}
async function confirm(){await $('[data-testid="confirm-project"]').scrollIntoView();await $('[data-testid="confirm-project"]').click();await browser.waitUntil(async()=>!await $('.project-dialog').isExisting(),{timeout:30000});}
async function activate(agent,mode='ربط'){await $('[data-testid="activate-project"]').click();await $('[data-testid="select-emil-color"]').waitForDisplayed();await choose('project-agent',agent);await choose('project-mode',mode);await $('[data-testid="select-emil-color"]').click();await $('[data-testid="preview-project"]').click();await $('[data-testid="project-preview"]').waitForDisplayed();}
async function unuse(){const before=tree(project);await $('[data-testid="unuse-all"]').waitForDisplayed();await $('[data-testid="unuse-all"]').click();await $('[data-testid="preview-project"]').click();await $('[data-testid="project-preview"]').waitForDisplayed();assert.deepEqual(tree(project),before);await confirm();await browser.waitUntil(async()=>await $$('.activation-row').length===0);}
function register(p){cli(['use','emil-ui-review','--project',p,'--agent','claude']);cli(['unuse','emil-ui-review','--project',p,'--agent','claude']);}
describe('Project operations through the native app — batches 6.4–6.5',()=>{
 let baseline;
 before(()=>{mkdirSync(project);writeFileSync(path.join(project,'README.md'),'ملف المشروع — keep exactly\n');writeFileSync(path.join(project,'.gitignore'),'node_modules/\n');const foreign=path.join(project,'.claude/skills/local-note');mkdirSync(foreign,{recursive:true});writeFileSync(path.join(foreign,'SKILL.md'),'---\nname: local-note\ndescription: Local project note\n---\nKeep this reference.\n');register(project);baseline=tree(project);});
 it('shows registered projects and local skills without changing any files',async()=>{
  const before=tree(project);await selectProject(project);await $('.local-row').waitForDisplayed();assert.match(await $('.local-list').getText(),/local-note/);assert.match(await $('[data-testid="project-empty-activations"]').getText(),/تفعيلات من كنانة.*فعّل مهارة/);assert.deepEqual(tree(project),before);
  const report=await invoke('projects',{selected:[]});assert.equal(report.find(p=>p.path===project).local[0].entry.name,'local-note');writeFileSync(path.join(evidence,'project-initial.json'),JSON.stringify({report,baseline},null,2)+'\n');
 });
 it('previews and cancels without writes; rejects unconfirmed and changed previews',async()=>{
  await activate('Claude Code');assert.deepEqual(tree(project),baseline);assert.equal(await $('[data-testid="gitignore-suggestions"]').isDisplayed(),true);assert.match(await $('[data-testid="reload-skills"]').getText(),/reload-skills/);await $('[data-testid="cancel-project"]').click();assert.deepEqual(tree(project),baseline);
  const request={action:'use',project,selections:['emil-color'],agent:'claude',mode:'link',include_secondary:[]};const preview=await invoke('preview_project',{request});
  for(const attempt of [{request,confirmed:false,token:preview.token,expected:'PROJECT_CONFIRMATION_REQUIRED'},{request:{...request,mode:'copy'},confirmed:true,token:preview.token,expected:'PROJECT_PREVIEW_CHANGED'}]){
   await invoke('start_project',{request:attempt.request,token:attempt.token,confirmed:attempt.confirmed});await browser.waitUntil(async()=>(await invoke('project_status')).done);const status=await invoke('project_status');assert.equal(status.error.code,attempt.expected);assert.deepEqual(tree(project),baseline);await invoke('cancel_project');
  }
  writeFileSync(path.join(evidence,'project-confirmation.json'),JSON.stringify({preview,cancel_no_writes:true,unconfirmed_rejected:true,changed_preview_rejected:true},null,2)+'\n');
 });
 it('activates all agents with links, captures Arabic and English, and restores the exact tree',async()=>{
  await activate('كل الوكلاء');await confirm();await browser.waitUntil(async()=>await $$('.activation-row').length===3);
  for(const folder of ['.claude','.agents'])assert.equal(lstatSync(path.join(project,folder,'skills/emil-color')).isSymbolicLink(),true);
  assert.match(await $('[data-testid="project-success"]').getText(),/reload-skills/);
  await browser.setWindowSize(1100,760);for(const locale of ['ar','en']){await browser.execute(()=>document.activeElement?.blur());assert.equal(await $('html').getAttribute('lang'),locale);await browser.saveScreenshot(path.join(evidence,`projects-${locale}.png`));await $('[data-testid="language"]').click();}
  const active=await invoke('projects',{selected:[]});await unuse();assert.deepEqual(tree(project),baseline);writeFileSync(path.join(evidence,'project-links-roundtrip.json'),JSON.stringify({before:baseline,active,after:tree(project),exact_tree_and_sha256_match:true},null,2)+'\n');
 });
 it('copies for all agents through the UI, then unuse restores the exact tree',async()=>{
  await activate('كل الوكلاء','نسخ');await confirm();await browser.waitUntil(async()=>await $$('.activation-row').length===3);for(const folder of ['.claude','.agents'])assert.equal(lstatSync(path.join(project,folder,'skills/emil-color')).isDirectory(),true);
  await unuse();assert.deepEqual(tree(project),baseline);writeFileSync(path.join(evidence,'project-copy-roundtrip.json'),JSON.stringify({before:baseline,after:tree(project),exact_tree_and_sha256_match:true},null,2)+'\n');
 });
 it('adopts only a byte-identical local copy after preview and preserves foreign content',async()=>{
  const local=path.join(project,'.claude/skills/emil-color');cpSync(path.join(vault,'skills/emil-color'),local,{recursive:true});const before=tree(project);await $('[data-testid="refresh-projects"]').click();await browser.waitUntil(async()=>await $$('.local-row').length===2);
  await $('[data-testid="adopt-project"]').click();await $('[data-testid="preview-project"]').click();await $('[data-testid="project-preview"]').waitForDisplayed();const candidates=await invoke('preview_project',{request:{action:'adopt',project,agent:'claude'}});assert.equal(candidates.candidates.filter(c=>c.eligible).length,1);assert.deepEqual(tree(project),before);await browser.saveScreenshot(path.join(evidence,'project-adopt-preview.png'));await confirm();assert.equal(lstatSync(local).isSymbolicLink(),true);await unuse();assert.deepEqual(tree(project),baseline);writeFileSync(path.join(evidence,'project-adoption.json'),JSON.stringify({candidates,before,after:tree(project),foreign_content_preserved:true,adopted_duplicate_explicitly_replaced:true},null,2)+'\n');
 });
 it('applies a bundle with an explicitly selected secondary skill and rejects unsupported Codex manual mode',async()=>{
  const input=path.join(base,'project-bundle.toml');writeFileSync(input,'name="project-design"\ntitle="تصميم المشروع"\nagents=["claude","codex"]\n[[skills]]\nid="emil-design-foundations"\nrole="primary"\ngroup="design"\n[[skills]]\nid="emil-ui-polish"\nrole="secondary"\ngroup="design"\n');cli(['bundle','create','--file',input]);
  await $('[data-testid="apply-project-bundle"]').click();await choose('project-bundle','تصميم المشروع');await $('[data-testid="secondary-emil-ui-polish"]').click();await choose('project-agent','Codex');await $('[data-testid="preview-project"]').click();await $('.project-dialog [data-testid="error"]').waitForDisplayed();assert.match(await $('.project-dialog [data-testid="error"]').getText(),/MANUAL/);await choose('project-agent','Claude Code');await $('[data-testid="preview-project"]').click();await $('[data-testid="project-preview"]').waitForDisplayed();assert.match(await $('[data-testid="project-preview"]').getText(),/يدوية/);const before=tree(project);await confirm();await browser.waitUntil(async()=>await $$('.activation-row').length===2);const actual=await invoke('projects',{selected:[]});const selected=actual.find(p=>p.path===project);assert.equal(selected.bundle,'project-design');assert.equal(selected.activations.find(a=>a.id==='emil-ui-polish').invocation,'manual');await unuse();assert.deepEqual(tree(project),before);writeFileSync(path.join(evidence,'project-bundle.json'),JSON.stringify({selected,restored:true},null,2)+'\n');
 });
 it('shows stale, broken, and modified targets and preserves a changed copy on failure',async()=>{
  const p=path.join(base,'state-project');mkdirSync(p);register(p);cli(['use','emil-design-foundations','--project',p,'--agent','claude']);cli(['review','emil-design-foundations','--accept']);await selectProject(p);await browser.waitUntil(async()=>(await $('[data-testid="activation-claude-emil-design-foundations"]').getAttribute('data-state'))==='stale');cli(['undo']);unlinkSync(path.join(p,'.claude/skills/emil-design-foundations'));await $('[data-testid="refresh-projects"]').click();await browser.waitUntil(async()=>(await $('[data-testid="activation-claude-emil-design-foundations"]').getAttribute('data-state'))==='broken');cli(['unuse','emil-design-foundations','--project',p]);
  cli(['use','emil-ui-review','--project',p,'--agent','codex','--mode','copy']);writeFileSync(path.join(p,'.agents/skills/emil-ui-review/LOCAL.txt'),'Do not remove\n');await $('[data-testid="refresh-projects"]').click();await browser.waitUntil(async()=>(await $('[data-testid="activation-codex-emil-ui-review"]').getAttribute('data-state'))==='modified');const before=tree(p);await $('[data-testid="unuse-all"]').click();await $('[data-testid="preview-project"]').click();await $('.project-dialog [data-testid="error"]').waitForDisplayed();assert.deepEqual(tree(p),before);writeFileSync(path.join(evidence,'project-modified-preserved.json'),JSON.stringify({before,after:tree(p),preserved:true,error:await $('.project-dialog [data-testid="error"]').getText()},null,2)+'\n');await $('[data-testid="cancel-project"]').click();assert.equal(existsSync(path.join(p,'.agents/skills/emil-ui-review/LOCAL.txt')),true);
 });
 it('shows audit evidence at 800 by 600 and writes only after desktop acknowledgment',async()=>{
  const fixture=path.join(base,'review-activation');
  mkdirSync(fixture);
  writeFileSync(path.join(fixture,'SKILL.md'),'---\nname: review-activation\ndescription: Audit demonstration.\n---\n```sh\nrm -rf "$TEST_ROOT"\n```\n');
  cli(['add',fixture,'--allow-risky']);
  await selectProject(project);
  await browser.setWindowSize(800,600);
  const before=tree(project);
  for(const locale of ['ar','en']){
   assert.equal(await $('html').getAttribute('lang'),locale);
   await $('[data-testid="activate-project"]').click();
   await $('[data-testid="select-review-activation"]').click();
   await $('[data-testid="preview-project"]').click();
   const review=$('[data-testid="project-risk-review"]');
   await review.waitForDisplayed();
   assert.match(await review.getText(),/SKILL.md/);
   assert.match(await review.getText(),/rm -rf/);
   assert.equal(await $('[data-testid="confirm-project"]').isEnabled(),false);
   await browser.saveScreenshot(path.join(evidence,`risky-activation-${locale}.png`));
   await $('[data-testid="cancel-project"]').click();
   assert.deepEqual(tree(project),before);
   await $('[data-testid="language"]').click();
  }
  await $('[data-testid="activate-project"]').click();
  await $('[data-testid="select-review-activation"]').click();
  await $('[data-testid="preview-project"]').click();
  await $('[data-testid="acknowledge-project-risk"]').waitForDisplayed();
  await $('[data-testid="acknowledge-project-risk"]').click();
  await confirm();
  await $('[data-testid="activation-claude-review-activation"]').waitForDisplayed();
  await unuse();
  assert.deepEqual(tree(project),before);
 });
});
