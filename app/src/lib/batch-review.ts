import type { BatchReview, PendingDescription } from "../bridge";
// Only proposals made for the current source can join a batch; the others are reviewed alone.
export function selectable(items:PendingDescription[]):string[] {return items.filter(item=>!item.source_changed).map(item=>item.id);}
// The batch is accepted only if the fresh preview shows exactly the texts the user read.
export function matchesShown(shown:PendingDescription[], preview:BatchReview, selected:string[]):boolean {
  if(preview.items.length!==selected.length)return false;
  const byId=new Map(shown.map(item=>[item.id,item]));
  return preview.items.every((item,i)=>{const seen=byId.get(item.id);return item.id===selected[i]&&!!seen&&!seen.source_changed&&seen.proposed===item.proposed&&seen.current===item.current;});
}
export function toggle(selected:string[], id:string, on:boolean):string[] {return on?(selected.includes(id)?selected:[...selected,id]):selected.filter(value=>value!==id);}
