import assert from 'node:assert/strict';
import {existsSync,readFileSync,writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('../../',import.meta.url));
async function invoke(command,params={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),command,params);}
describe('Vault location warnings in Settings',()=>{
 it('warns in both languages, preserves the selected vault, and warns again for an existing Documents vault',async()=>{
  const home=process.env.KINANA_UI_HOME,evidence=process.env.KINANA_UI_EVIDENCE,results=[];
  await $('[data-testid="nav-settings"]').click();await $('[data-testid="settings-vault-path"]').waitForDisplayed();
  const before=await invoke('desktop_settings'),config=readFileSync(path.join(before.vault,'box.toml'));
  for(const locale of ['ar','en']){
   assert.equal(await $('html').getAttribute('lang'),locale);
   for(const [relative,code] of [['Documents/مخزن عربي','VAULT_LOCATION_DOCUMENTS'],['Desktop/مخزن عربي','VAULT_LOCATION_DESKTOP'],['Library/Mobile Documents/com~apple~CloudDocs/مخزن عربي','VAULT_LOCATION_ICLOUD']]){
    await $('[data-testid="settings-vault-path"]').setValue(path.join(home,relative));
    const warning=await $(`[data-testid="vault-location-warnings"] [data-reason-code="${code}"]`);await warning.waitForDisplayed();
    const text=await warning.getText();assert.match(text,locale==='ar'?/تنبيه/:/Warning/);assert.match(text,/KinanaVault/);
    results.push({locale,relative,code,text});await browser.saveScreenshot(path.join(evidence,`settings-${code}-${locale}.png`));
    assert.equal(existsSync(path.join(home,relative)),false);
   }
   await $('[data-testid="language"]').click();
  }
  assert.equal((await invoke('desktop_settings')).vault,before.vault);
  assert.deepEqual(readFileSync(path.join(before.vault,'box.toml')),config);
  assert.equal(existsSync(path.join(home,'.kinana/desktop.toml')),false);
  const existing=path.join(home,'Documents/مخزن موجود');
  const init=spawnSync(process.env.KINANA_TEST_CLI,['--vault',existing,'--json','init'],{env:{...process.env,HOME:home},encoding:'utf8'});
  assert.equal(init.status,0,init.stdout+init.stderr);
  const existingConfig=readFileSync(path.join(existing,'box.toml'));
  await $('[data-testid="settings-vault-path"]').setValue(existing);
  await $('[data-reason-code="VAULT_LOCATION_DOCUMENTS"]').waitForDisplayed();
  await $('[data-testid="settings-select-vault"]').click();await $('[data-testid="settings-preview"]').waitForDisplayed();
  const preview=await $('[data-testid="settings-preview"]').getText();assert.match(preview,/Documents/);assert.match(preview,/تنبيه/);
  assert.equal((await invoke('desktop_settings')).vault,before.vault);
  const plan=await invoke('preview_settings',{action:{kind:'select_vault',path:existing}});assert.deepEqual(plan.diagnostic_codes,['VAULT_LOCATION_DOCUMENTS']);
  await $('[data-testid="confirm-settings"]').click();await browser.waitUntil(async()=>!await $('.settings-confirm-dialog').isExisting());
  await browser.waitUntil(async()=>(await invoke('desktop_settings')).vault===existing);
  await $('[data-testid="vault-location-warnings"] [data-reason-code="VAULT_LOCATION_DOCUMENTS"]').waitForDisplayed();
  const after=await invoke('desktop_settings');assert.deepEqual(after.vault_location_codes,['VAULT_LOCATION_DOCUMENTS']);
  assert.deepEqual(readFileSync(path.join(existing,'box.toml')),existingConfig);
  assert.deepEqual(readFileSync(path.join(before.vault,'box.toml')),config);
  assert.equal(existsSync(path.join(home,'KinanaVault')),false);
  await browser.saveScreenshot(path.join(evidence,'settings-existing-documents-ar.png'));
  writeFileSync(path.join(evidence,'settings-location-warnings.json'),JSON.stringify({before,after,results,preview,existing_vaults_unchanged:true,only_explicit_selection_saved:true},null,2)+'\n');
 });
});
