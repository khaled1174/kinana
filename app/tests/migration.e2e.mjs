import assert from 'node:assert/strict';
import {readFileSync,readdirSync,lstatSync,existsSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import path from 'node:path';
const home=process.env.KINANA_UI_HOME,evidence=process.env.KINANA_UI_EVIDENCE;
function snapshot(root){return Object.fromEntries(readdirSync(root,{recursive:true}).sort().map(name=>{
 const p=path.join(root,name),s=lstatSync(p);assert.equal(s.isSymbolicLink(),false);
 return [name,{mode:s.mode,ino:s.ino,hash:s.isFile()?createHash('sha256').update(readFileSync(p)).digest('hex'):null}];
}));}
describe('Legacy state in the real Kinana WebView',()=>{
 it('reads the old vault, explains migration in both languages, and blocks writes',async()=>{
  assert.ok(process.env.KINANA_UI_LEGACY);const before=snapshot(path.join(home,'.skillsbox')),vaultBefore=snapshot(path.join(home,'SkillsBox'));
  await $('[data-testid="skill-emil-color"]').waitForDisplayed();
  assert.equal(await browser.execute(()=>document.title),'Kinana');assert.equal(await $('.brand strong').getText(),'Kinana');
  await $('[data-testid="pin-emil-color"]').click();
  await browser.waitUntil(async()=>await $('[role="alert"]').isExisting());
  const error=await $('[role="alert"]').getText();assert.match(error,/للقراءة فقط/);assert.doesNotMatch(error,/Legacy|read-only/);
  await $('[data-testid="nav-settings"]').click();await $('[data-testid="settings-tab-general"]').waitForDisplayed();
  await $('[data-testid="migration-required"]').waitForDisplayed();
  assert.match(await $('[data-testid="migration-required"]').getText(),/kinana migrate --dry-run/);
  await browser.saveScreenshot(path.join(evidence,'migration-required-ar.png'));
  await $('[data-testid="language"]').click();assert.match(await $('[data-testid="migration-required"]').getText(),/read-only/);
  await browser.saveScreenshot(path.join(evidence,'migration-required-en.png'));
  assert.equal(existsSync(path.join(home,'.kinana')),false);assert.deepEqual(snapshot(path.join(home,'.skillsbox')),before);assert.deepEqual(snapshot(path.join(home,'SkillsBox')),vaultBefore);
  writeFileSync(path.join(evidence,'native-migration.json'),JSON.stringify({home,brand:'Kinana',title:'Kinana',legacy_readable:true,localized_migration_notice:true,mutation_rejected:true,old_state_bytes_modes_identities_unchanged:true,new_state_not_created:true},null,2)+'\n');
 });
});
