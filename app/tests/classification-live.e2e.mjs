import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
const evidence=process.env.KINANA_UI_EVIDENCE,vault=process.env.KINANA_LIVE_VAULT,ids=process.env.KINANA_LIVE_IDS.split(','),max=Number(process.env.KINANA_LIVE_MAX_CALLS);
async function invoke(command,params={}){return browser.tauri.execute(async({core},command,params)=>core.invoke(command,params),command,params);}
const git=args=>spawnSync('git',['-C',vault,...args],{encoding:'utf8'}).stdout.trim();
describe('Live classify all (real claude, owner permission)',()=>{
 it(`classifies ${ids.length} skills with at most ${max} model calls and records agent_calls exactly`,async()=>{
  await $('[data-testid="nav-settings"]').waitForDisplayed({timeout:30000});
  const preview=await invoke('preview_classify',{all:true});
  assert.equal(preview.selected,ids.length);assert.equal(preview.estimated_agent_calls,ids.length);
  // Retries are bounded by core (two attempts per skill), so the cap is a check, not a stop.
  assert.ok(2*ids.length<=max,'the permission must cover the retry ceiling');
  const commits=Number(git(['rev-list','--count','HEAD']));const started=new Date().toISOString();
  await invoke('start_classify',{all:true,estimate:preview.estimated_tokens,confirmed:true});
  const samples=[];let status;
  await browser.waitUntil(async()=>{status=await invoke('classify_status');samples.push(status.progress);return status.done;},{timeout:540000,interval:500});
  await invoke('cancel_classify');
  const report=status.report,error=status.error;
  const result={started_at:started,finished_at:new Date().toISOString(),permission:{skills:ids.length,expected_calls:ids.length,max_calls:max},
   agent_calls:report?.agent_calls??null,estimated_agent_calls:preview.estimated_agent_calls,estimated_tokens:preview.estimated_tokens,
   stopped:report?.stopped??null,committed:report?.committed??null,error,new_commits:Number(git(['rev-list','--count','HEAD']))-commits,
   items:(report?.items||[]).map(i=>({id:i.id,classified_by:i.classified_by,proposal_created:i.proposal_created,attempts:i.attempts,warnings:i.warnings})),
   progress_samples:samples.length,vault_clean:git(['status','--porcelain'])===''};
  writeFileSync(path.join(evidence,'live-classify-all.json'),JSON.stringify(result,null,2)+'\n');
  console.log('LIVE agent_calls='+result.agent_calls);
  assert.equal(error,null,JSON.stringify(error));
  assert.ok(result.agent_calls<=max,`agent_calls ${result.agent_calls} > ${max}`);
  assert.ok(report.committed);assert.equal(result.new_commits,1);assert.ok(result.vault_clean);
 });
});
