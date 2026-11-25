import { test } from "node:test";
import assert from "node:assert/strict";
import { setDefinition, setRules, restoreRules } from "../src/lib/taxonomy-draft.ts";
import { categoryExplanations, stageExplanations, ruleExplanations } from "../src/lib/taxonomy-ar.ts";
test("an empty or default text restores the default instead of storing it",()=>{
  let d=setDefinition({},"categories","mobile","Use for phones.","Default mobile.");
  assert.deepEqual(d.categories,{mobile:"Use for phones."});
  assert.deepEqual(setDefinition(d,"categories","mobile","","Default mobile.").categories,{});
  assert.deepEqual(setDefinition(d,"categories","mobile","Default mobile.","Default mobile.").categories,{});
  assert.deepEqual(setDefinition({},"stacks","react","Use when hooks matter.",null).stacks,{react:"Use when hooks matter."});
});
test("rules equal to the defaults are not stored; an empty box sends none",()=>{
  const defaults=["a","b"];
  assert.equal("rules" in setRules({},"a\n b \n",defaults),false);
  assert.deepEqual(setRules({},"c",defaults).rules,["c"]);
  assert.deepEqual(setRules({},"",defaults).rules,[]);
  assert.equal("rules" in restoreRules({rules:["c"]}),false);
});
test("every default value has an Arabic explanation",()=>{
  for(const c of ["mobile","web-frontend","backend-api","data-db","devops","testing-qa","planning-specs","design-ui","docs-writing","content-media","automation-agents","security","other"])assert.ok(categoryExplanations[c],c);
  for(const s of ["plan","design","build","test","review","ship"])assert.ok(stageExplanations[s],s);
  assert.equal(ruleExplanations.length,6);
});
