import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {AUDIT_RULES_VERSION,auditRulesState,upgradeRows} from '../src/lib/audit-rules.ts';
test('the app and core agree on the audit rules version',()=>{
 const core=readFileSync(new URL('../../crates/core/src/audit.rs',import.meta.url),'utf8');
 const match=core.match(/pub const RULES_VERSION: u32 = (\d+);/);
 assert.ok(match,'RULES_VERSION not found in core');
 assert.equal(Number(match[1]),AUDIT_RULES_VERSION);
});
test('stored audits are legacy, older or current by their rules version',()=>{
 assert.equal(auditRulesState(0),'legacy');
 assert.equal(auditRulesState(2),'older');
 assert.equal(auditRulesState(AUDIT_RULES_VERSION),'current');
});
test('the preview lists skills whose level goes up first, each group by id',()=>{
 const rows=upgradeRows({rules_version:3,older:4,unchanged:0,
  lowered:[{id:'b-low',from:'risky',to:'clean'},{id:'a-low',from:'risky',to:'review'}],
  raised:[{id:'z-up',from:'clean',to:'risky'},{id:'m-up',from:'review',to:'risky'}]});
 assert.deepEqual(rows.map(r=>[r.direction,r.change.id]),[['raised','m-up'],['raised','z-up'],['lowered','a-low'],['lowered','b-low']]);
});
