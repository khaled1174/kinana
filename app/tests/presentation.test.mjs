import {test} from 'node:test';
import assert from 'node:assert/strict';
import {descriptionDiff,skillCount} from '../src/lib/presentation.ts';
test('Arabic plural forms use singular, dual, few and many/other counts',()=>{
 const expectedForms=[[0,'مهارة'],[3,'مهارات'],[7,'مهارات'],[10,'مهارات'],[11,'مهارة'],[99,'مهارة'],[100,'مهارة'],[102,'مهارة']];
 const number=new Intl.NumberFormat('ar');
 for(const [count,noun] of expectedForms)assert.equal(skillCount(count,'ar'),`${number.format(count)} ${noun}`);
 assert.equal(skillCount(1,'ar'),'مهارة واحدة');assert.equal(skillCount(2,'ar'),'مهارتان');
 assert.equal(skillCount(1,'en'),'1 skill');assert.equal(skillCount(2,'en'),'2 skills');
});
test('word diff preserves complete original/proposed strings and marks only changes',()=>{
 for(const [a,b,removed,added] of [['Read old files.','Read new files.','old','new'],['اقرأ الملف القديم.','اقرأ الملف الجديد.','القديم','الجديد'],['Keep  whitespace\nintact.','Keep  all whitespace\nintact.','','all '],['same','same','',''],['','hello','','hello'],['hello','','hello','']]) {
  const d=descriptionDiff(a,b);assert.equal(d.original.map(p=>p.text).join(''),a);assert.equal(d.proposed.map(p=>p.text).join(''),b);
  assert.equal(d.original.filter(p=>p.changed).map(p=>p.text).join(''),removed);assert.equal(d.proposed.filter(p=>p.changed).map(p=>p.text).join(''),added);
 }
});
test('synthetic Claude proposal remains unabridged, including its final sentence',async()=>{
 const {readFileSync}=await import('node:fs');const fixture=JSON.parse(readFileSync(new URL('../../tests/fixtures/public-release/presentation-proposal.json',import.meta.url)));
 const d=descriptionDiff(fixture.original,fixture.proposal.improved_description);assert.equal(d.original.map(p=>p.text).join(''),fixture.original);assert.equal(d.proposed.map(p=>p.text).join(''),fixture.proposal.improved_description);assert.ok(d.original.some(p=>p.changed));assert.ok(d.proposed.some(p=>p.changed));
});
