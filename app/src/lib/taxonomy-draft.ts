import type { Definitions } from "../bridge";
// One edit of a definition. An empty text or the default text means "use the default".
export function setDefinition(draft:Definitions, kind:"categories"|"stacks"|"stages", value:string, text:string, fallback:string|null):Definitions {
  const table={...(draft[kind]??{})};
  if(!text.trim()||text===fallback)delete table[value];else table[value]=text;
  return {...draft,[kind]:table};
}
// The rules text area holds one rule per line; the defaults are sent as "no change".
export function setRules(draft:Definitions, text:string, defaults:string[]):Definitions {
  const rules=text.split("\n").map(line=>line.trim()).filter(Boolean);
  const same=rules.length===defaults.length&&rules.every((rule,i)=>rule===defaults[i]);
  const next={...draft};
  if(same)delete next.rules;else next.rules=rules;
  return next;
}
export function restoreRules(draft:Definitions):Definitions {const next={...draft};delete next.rules;return next;}
