import {test} from 'node:test';
import assert from 'node:assert/strict';
import {planScan,scanSelection,emptyChoice} from '../src/lib/scan-plan.ts';
const audit=(level='clean')=>({level,findings:[],rules_version:1,details:[]});
const item=(path,status,name,hash,id=null,level)=>({id,name,path,status,content_hash:hash,audit:audit(level),selected:true,error:null});
// The owner's first run in small: new, a duplicate of a new skill, a duplicate of a vault skill,
// a name found twice with different contents, a drift from the vault's copy, and an invalid folder.
const report={source:'scan',dry_run:true,requires_preparation:false,imported:[],duplicates:[],variants:[],committed:false,warnings:[],items:[
 item('/a/color','new','emil-color','h1','emil-color'),
 item('/b/color','duplicate','emil-color','h1','emil-color'),
 item('/a/perf','new','perf','h2','perf',"risky"),
 item('/a/design','new','design','h3','design'),
 item('/b/design','variant','design','h4'),
 item('/c/design','duplicate','design','h3','design'),
 item('/a/owned','duplicate','owned','h5','owned'),
 item('/b/owned','variant','owned','h6','owned'),
 {...item('/a/broken','broken',null,null),error:'Missing name'},
]};
test('only new skills are selected; versions wait for a choice; duplicates follow their original',()=>{
 const plan=planScan(report);
 assert.deepEqual(plan.fresh.map(i=>i.path),['/a/color','/a/perf']);
 assert.deepEqual(plan.conflicts.map(c=>[c.name,c.versions.map(v=>v.items.map(i=>i.path))]),[['design',[['/a/design','/c/design'],['/b/design']]]]);
 assert.deepEqual(plan.duplicates.map(i=>i.path),['/b/color','/a/owned']);
 assert.deepEqual(plan.drifted.map(i=>i.path),['/b/owned']);
 assert.deepEqual(plan.broken.map(i=>i.path),['/a/broken']);
 const s=scanSelection(plan,emptyChoice());
 assert.deepEqual(s.select,['/a/color','/a/perf','/b/color','/a/owned']);
 assert.deepEqual(s.primary,[]);
 assert.equal(s.added,2);assert.equal(s.locations,2);assert.equal(s.risky,1);
});
test('a chosen primary sends every version of its name; clearing a new skill drops its duplicates',()=>{
 const plan=planScan(report);
 const s=scanSelection(plan,{unchecked:['/a/color'],primary:{design:'/b/design'},drift:['/b/owned']});
 assert.deepEqual(s.select,['/a/perf','/a/owned','/a/design','/c/design','/b/design','/b/owned']);
 assert.deepEqual(s.primary,['/b/design']);
 assert.equal(s.added,2);assert.equal(s.locations,4);
 // A remembered primary that the new scan no longer has is not sent.
 assert.deepEqual(scanSelection(plan,{...emptyChoice(),primary:{design:'/gone'}}).primary,[]);
});
