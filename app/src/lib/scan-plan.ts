import type {ImportReport} from "../bridge";

export type ScanItem = ImportReport["items"][number];
export type ScanVersion = {hash:string;items:ScanItem[]};
/** A scan review sorted by what each discovery needs from the user. */
export type ScanPlan = {
 /** New skills, selected unless the user clears them. */
 fresh: ScanItem[];
 /** Names found with different contents and no copy in the vault: the user picks one version, none is picked for them. */
 conflicts: {name:string;versions:ScanVersion[]}[];
 /** The same content as a new skill or a vault skill: recorded as one more location, never copied twice. */
 duplicates: ScanItem[];
 /** Different from the vault's copy of the same name: recorded as a drifted location only when chosen. */
 drifted: ScanItem[];
 broken: ScanItem[];
};
/** What the user changed from the defaults; kept while the same scan stays open. */
export type ScanChoice = {unchecked:string[];primary:Record<string,string>;drift:string[]};
export const emptyChoice=():ScanChoice=>({unchecked:[],primary:{},drift:[]});

export function planScan(report:ImportReport):ScanPlan {
 const valid=report.items.filter(item=>item.status!=="broken"&&item.name!==null);
 // A variant without a vault owner has no ID: its name has no copy in the vault yet.
 const conflictNames=new Set(valid.filter(item=>item.status==="variant"&&item.id===null).map(item=>item.name as string));
 const conflicts=[...conflictNames].sort().map(name=>{
  const versions=new Map<string,ScanItem[]>();
  for(const item of valid)if(item.name===name){const hash=item.content_hash??item.path;versions.set(hash,[...(versions.get(hash)??[]),item]);}
  return {name,versions:[...versions].map(([hash,items])=>({hash,items}))};
 });
 const outside=(item:ScanItem)=>!conflictNames.has(item.name as string);
 return {
  fresh:valid.filter(item=>item.status==="new"&&outside(item)),
  conflicts,
  duplicates:valid.filter(item=>item.status==="duplicate"&&outside(item)),
  drifted:valid.filter(item=>item.status==="variant"&&item.id!==null&&outside(item)),
  broken:report.items.filter(item=>item.status==="broken"||item.name===null),
 };
}

/** The engine request for a plan and choice, and the counts the review shows. */
export function scanSelection(plan:ScanPlan,choice:ScanChoice){
 const fresh=plan.fresh.filter(item=>!choice.unchecked.includes(item.path));
 const freshHashes=new Map(plan.fresh.map(item=>[item.content_hash,item.path]));
 const chosenHashes=new Set(fresh.map(item=>item.content_hash));
 // A duplicate of a new skill follows that skill; a duplicate of a vault skill is always recorded.
 const duplicates=plan.duplicates.filter(item=>!freshHashes.has(item.content_hash)||chosenHashes.has(item.content_hash));
 const primary:string[]=[],versions:ScanItem[]=[];
 for(const conflict of plan.conflicts){
  const path=choice.primary[conflict.name];
  const items=conflict.versions.flatMap(version=>version.items);
  if(path&&items.some(item=>item.path===path)){primary.push(path);versions.push(...items);}
 }
 const drift=plan.drifted.filter(item=>choice.drift.includes(item.path));
 const all=[...fresh,...duplicates,...versions,...drift];
 return {
  select:all.map(item=>item.path),
  primary,
  added:fresh.length+primary.length,
  locations:all.length-fresh.length-primary.length,
  risky:all.filter(item=>item.audit.level==="risky").length,
 };
}
