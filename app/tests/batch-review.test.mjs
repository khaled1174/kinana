import { test } from "node:test";
import assert from "node:assert/strict";
import { matchesShown, selectable, toggle } from "../src/lib/batch-review.ts";
const shown=[{id:"a",name:"a",current:"old a",proposed:"new a",source_changed:false},{id:"b",name:"b",current:"old b",proposed:"new b",source_changed:true},{id:"c",name:"c",current:"old c",proposed:"new c",source_changed:false}];
const preview=(items)=>({decision:"accept",items,preview_token:"r1.x",requires_confirmation:true,dry_run:true,committed:false});
test("a proposal for the previous source is never selectable",()=>{assert.deepEqual(selectable(shown),["a","c"]);});
test("toggle keeps the order of selection and never duplicates",()=>{let s=toggle([],"c",true);s=toggle(s,"a",true);s=toggle(s,"a",true);assert.deepEqual(s,["c","a"]);assert.deepEqual(toggle(s,"c",false),["a"]);});
test("the fresh preview must show exactly the texts that were read",()=>{
  const same=preview([{id:"c",original:"",current:"old c",proposed:"new c",active:"original"},{id:"a",original:"",current:"old a",proposed:"new a",active:"original"}]);
  assert.equal(matchesShown(shown,same,["c","a"]),true);
  const changed=preview([{id:"c",original:"",current:"old c",proposed:"newer c",active:"original"},{id:"a",original:"",current:"old a",proposed:"new a",active:"original"}]);
  assert.equal(matchesShown(shown,changed,["c","a"]),false);
  assert.equal(matchesShown(shown,same,["a","c"]),false);
  assert.equal(matchesShown(shown,preview([same.items[0]]),["c","a"]),false);
  const blocked=preview([{id:"b",original:"",current:"old b",proposed:"new b",active:"original"}]);
  assert.equal(matchesShown(shown,blocked,["b"]),false);
});
