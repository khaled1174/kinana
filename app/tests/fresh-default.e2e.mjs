import assert from 'node:assert/strict';
import {existsSync,writeFileSync} from 'node:fs';
import path from 'node:path';
describe('Kinana fresh-install default',()=>{
 it('offers KinanaVault without creating state or a vault',async()=>{
  const home=process.env.KINANA_UI_HOME;
  await $('[data-testid="onboarding"]').waitForDisplayed();
  const settings=await browser.tauri.execute(async({core})=>core.invoke('desktop_settings'));
  assert.equal(settings.vault,path.join(home,'KinanaVault'));
  assert.equal(settings.migration_required,false);
  assert.equal(await $('[data-testid="settings-vault-path"]').getValue(),settings.vault);
  for(const name of ['.kinana','.skillsbox','.kinana-write.lock','Documents','KinanaVault'])assert.equal(existsSync(path.join(home,name)),false,name);
  assert.equal((await $('[data-testid="vault-location-warnings"]').getText()).trim(),'');
  writeFileSync(path.join(process.env.KINANA_UI_EVIDENCE,'fresh-default.json'),JSON.stringify({settings,read_only:true},null,2)+'\n');
 });
 it('warns in first-run setup in both languages without creating or moving files',async()=>{
  const home=process.env.KINANA_UI_HOME, results=[];
  for(const locale of ['ar','en']){
   for(const [relative,code] of [['Documents/مخزن عربي','VAULT_LOCATION_DOCUMENTS'],['Desktop/مخزن عربي','VAULT_LOCATION_DESKTOP'],['Library/Mobile Documents/com~apple~CloudDocs/مخزن عربي','VAULT_LOCATION_ICLOUD']]){
    await $('[data-testid="settings-vault-path"]').setValue(path.join(home,relative));
    const warning=await $(`[data-testid="vault-location-warnings"] [data-reason-code="${code}"]`);await warning.waitForDisplayed();
    const text=await warning.getText();assert.match(text,locale==='ar'?/تنبيه/:/Warning/);assert.match(text,/KinanaVault/);
    await browser.saveScreenshot(path.join(process.env.KINANA_UI_EVIDENCE,`onboarding-${code}-${locale}.png`));results.push({locale,relative,code,text});
   }
   await $('[data-testid="language"]').click();
  }
  await $('[data-testid="settings-vault-path"]').setValue(path.join(home,'Documents-other/مخزن عربي'));
  await browser.waitUntil(async()=>!(await $('[data-testid="vault-location-warnings"]').getText()).trim());
  await $('[data-testid="settings-vault-path"]').setValue(path.join(home,'KinanaVault'));
  for(const name of ['.kinana','.skillsbox','.kinana-write.lock','Documents','Desktop','Library','KinanaVault'])assert.equal(existsSync(path.join(home,name)),false,name);
  writeFileSync(path.join(process.env.KINANA_UI_EVIDENCE,'onboarding-location-warnings.json'),JSON.stringify({results,read_only:true,no_existing_vault_moved:true},null,2)+'\n');
 });
});
