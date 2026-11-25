import {test} from 'node:test';
import assert from 'node:assert/strict';
import {auditGroups,placesLabel} from '../src/lib/audit-groups.ts';
const d=(rule,path,line,level)=>({rule,path,line,end_line:line,level,strength:level==='risky'?'strong':'weak',reason_code:'X',excerpt:null});
test('reasons are grouped by rule and file, risky groups and lines first',()=>{
 const groups=auditGroups([
  d('network','README.md',3,'review'),
  d('secrets','scripts/a.sh',9,'review'),
  d('secrets','scripts/a.sh',4,'risky'),
  d('secrets','README.md',59,'risky'),
  d('secrets','scripts/a.sh',2,'risky'),
 ]);
 assert.deepEqual(groups.map(g=>[g.rule,g.path,g.level,g.details.map(x=>x.line)]),[
  ['secrets','scripts/a.sh','risky',[2,4,9]],
  ['secrets','README.md','risky',[59]],
  ['network','README.md','review',[3]],
 ]);
});
test('place counts read naturally in Arabic and English',()=>{
 assert.equal(placesLabel(1,'ar'),'موضع واحد');
 assert.equal(placesLabel(2,'ar'),'موضعان');
 const n=x=>new Intl.NumberFormat('ar').format(x);
 assert.equal(placesLabel(3,'ar'),`${n(3)} مواضع`);
 assert.equal(placesLabel(13,'ar'),`${n(13)} موضعًا`);
 assert.equal(placesLabel(1,'en'),'1 place');
 assert.equal(placesLabel(13,'en'),'13 places');
});
