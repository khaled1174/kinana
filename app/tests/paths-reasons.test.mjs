import assert from 'node:assert/strict';
import {test} from 'node:test';
import {displayPath} from '../src/lib/paths.ts';
import {errorText,reasonText} from '../src/reasons.ts';
test('paths use project-relative and home-relative forms without matching sibling prefixes',()=>{
 const home='/Users/owner',project=home+'/مشروع عربي';assert.equal(displayPath(project+'/.claude/skills/مهارة',home,project),'.claude/skills/مهارة');assert.equal(displayPath(project,home,project),'.');assert.equal(displayPath(home+'/SkillsBox',home),'~/SkillsBox');assert.equal(displayPath('/Users/owner-other/file',home),'/Users/owner-other/file');assert.equal(displayPath(home+'/مشروع عربي آخر/file',home,project),'~/مشروع عربي آخر/file');assert.equal(displayPath('C:\\Users\\me\\work\\a','C:\\Users\\me','C:\\Users\\me\\work'),'a');
});
test('unknown and known reason codes never expose the English core message in Arabic',()=>{
 const message='ENGLISH_CORE_MESSAGE_MUST_NOT_APPEAR';const error={code:'NEW_FUTURE_ERROR',reason_code:'NEW_FUTURE_REASON',message};assert.ok(!errorText(error,'ar').includes(message));assert.match(errorText(error,'ar'),/[\u0600-\u06ff]/);assert.match(errorText({code:'IO_ERROR',reason_code:'IO_PERMISSION_DENIED',message},'ar'),/صلاحيات/);assert.match(reasonText('AUDIT_INSTRUCTION_OVERRIDE_WEAK','ar'),/مبهم/);assert.match(reasonText('AUDIT_NETWORK_STRONG','ar'),/فعلي/);
});

test('vault location warnings are translated without dropping the warning or local alternative',()=>{
 for(const code of ['VAULT_LOCATION_DOCUMENTS','VAULT_LOCATION_DESKTOP','VAULT_LOCATION_ICLOUD']){
  assert.match(reasonText(code,'ar'),/تنبيه/);assert.match(reasonText(code,'en'),/Warning/);
  assert.match(reasonText(code,'ar'),/KinanaVault/);assert.match(reasonText(code,'en'),/KinanaVault/);
 }
});

test("names and path labels use visible Unicode escapes, preserving normal Arabic",async()=>{
 const {visibleName}=await import("../src/lib/paths.ts");
 assert.equal(visibleName("مراجعة الواجهة"),"مراجعة الواجهة");
 for(const cp of [0x202e,0x200b,0x2060,0x600,0x1680,0x115f,0xfe0f,0xe0041]){
   assert.equal(visibleName("م"+String.fromCodePoint(cp)+"م"),"م\\u{"+cp.toString(16)+"}م");
 }
 assert.equal(visibleName(" a\\b "), "\\u{20}a\\\\b\\u{20}");
});

test("bundle editor propagates member-load failures and rejects another vault on retry",async()=>{
 const {loadBundleSkills}=await import("../src/lib/bundle-skills.ts");
 const wanted="/vault/one";
 await assert.rejects(loadBundleSkills(wanted,async()=>{throw new Error("catalog unavailable");}),/catalog unavailable/);
 await assert.rejects(loadBundleSkills(wanted,async()=>({vault:"/vault/two",items:[]})),e=>e.code==="VAULT_CHANGED");
 assert.deepEqual(await loadBundleSkills(wanted,async()=>({vault:wanted,items:[{skill:{id:"member"}}]})),[{id:"member"}]);
});

test("0.3 stop codes are translated, and only a missing Claude says keywords were used",()=>{
 // Owner, 2026-11-18: a failing Claude stops or moves to Pi; it never becomes keywords.
 for(const code of ['AGENT_CLAUDE_UNAVAILABLE','AGENT_AUTO_UNAVAILABLE','AGENT_PI_UNAVAILABLE','AGENT_CODEX_UNSUPPORTED']){
  assert.match(reasonText(code,'ar'),/[؀-ۿ]/);assert.doesNotMatch(reasonText(code,'en'),/unknown/i);
 }
 for(const code of ['AGENT_TOOLS_NOT_ISOLATED','AGENT_CAPABILITY_UNPROVEN','AGENT_EXIT_FAILED','AGENT_TIMEOUT','INVALID_AGENT_PROPOSAL']){
  assert.doesNotMatch(reasonText(code,'ar'),/الكلمات المفتاحية/);assert.doesNotMatch(reasonText(code,'en'),/keyword/i);
 }
 // Formal Arabic for the Codex refusal (no colloquial forms).
 assert.doesNotMatch(reasonText('AGENT_CODEX_UNSUPPORTED','ar'),/ما نقدر|إنه ما/);
 assert.match(reasonText('AGENT_CODEX_UNSUPPORTED','ar'),/لا تستطيع/);
});

test("Pi's model stop has a translated text that names Pi's settings, not Claude",()=>{
 // Owner, 2026-11-19, live test report, item 1d.
 assert.match(reasonText('AGENT_MODEL_UNAVAILABLE','ar'),/[؀-ۿ]/);
 assert.match(reasonText('AGENT_MODEL_UNAVAILABLE','en'),/Pi settings/);
 assert.doesNotMatch(reasonText('AGENT_MODEL_UNAVAILABLE','en'),/Claude Code/);
});

test("Pi's connection stop and its extra instructions file have their own translated texts",()=>{
 // Owner, 2026-11-19, items 3 and 4.
 for(const code of ['AGENT_CONNECTION_FAILED','PI_APPEND_SYSTEM_FILE']){
  assert.match(reasonText(code,'ar'),/[؀-ۿ]/);assert.notEqual(reasonText(code,'en'),code);
  assert.doesNotMatch(reasonText(code,'ar'),/الكلمات المفتاحية/);assert.doesNotMatch(reasonText(code,'en'),/keyword/i);
 }
 assert.doesNotMatch(reasonText('AGENT_CONNECTION_FAILED','en'),/tool/i);
 // It says what blocks Pi and how to get past it.
 assert.match(reasonText('PI_APPEND_SYSTEM_FILE','en'),/APPEND_SYSTEM\.md.*Rename or move/s);
 assert.match(reasonText('PI_APPEND_SYSTEM_FILE','ar'),/APPEND_SYSTEM\.md.*غيّر اسم الملف أو انقله/s);
});
