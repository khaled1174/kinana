import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import { api, asError, type BatchReview, type CoreError, type PendingDescription } from "../bridge";
import type { Text } from "../i18n";
import { visibleName } from "../lib/paths";
import { matchesShown, selectable, toggle } from "../lib/batch-review";
import { reasonText } from "../reasons";
import { Button } from "./ui/button";
import { Checkbox } from "./ui/checkbox";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { DescriptionReview } from "./DescriptionReview";
import { ErrorNotice, EmptyNotice } from "./common";
const words={
 ar:{title:"مراجعة الأوصاف المقترحة",body:"اقرأ كل وصف، وحدد ما تعتمده. تُعتمد المحددة معًا في تغيير واحد، ويتراجع عنها «تراجع» مرة واحدة.",all:"تحديد الكل",none:"إلغاء التحديد",accept:(n:number)=>`اعتماد المحدد (${new Intl.NumberFormat("ar").format(n)})`,confirm:(n:number)=>`اعتماد ${new Intl.NumberFormat("ar").format(n)} وصف؟`,confirmBody:"يتغيّر الوصف النشط في dist لهذه المهارات فقط، وتبقى الأصول كما هي. الأوصاف غير المحددة تبقى تنتظر.",alone:"اقتُرح هذا الوصف للمصدر السابق؛ راجعه وحده من تفاصيل المهارة.",changed:"تغيّرت الأوصاف بعد عرضها؛ أُعيد تحميل القائمة. راجعها ثم أعد التحديد.",empty:"لا أوصاف تنتظر المراجعة"},
 en:{title:"Review proposed descriptions",body:"Read each description and select the ones to approve. Selected descriptions are approved together in one change, and one Undo reverts them.",all:"Select all",none:"Clear selection",accept:(n:number)=>`Approve selected (${n})`,confirm:(n:number)=>`Approve ${n} description${n===1?"":"s"}?`,confirmBody:"Only the active description in dist changes for these skills; originals stay intact. Unselected descriptions keep waiting.",alone:"This description was proposed for the previous source; review it alone from the skill details.",changed:"The descriptions changed after they were shown; the list was reloaded. Review it and select again.",empty:"No descriptions are waiting for review"},
};
export function BatchReviewDialog({t,onClose,onSaved}:{t:Text;onClose:()=>void;onSaved:()=>void}) {
 const w=words[t.locale as "ar"|"en"];
 const [items,setItems]=useState<PendingDescription[]>();const [selected,setSelected]=useState<string[]>([]);
 const [error,setError]=useState<CoreError>();const [busy,setBusy]=useState(false);const [preview,setPreview]=useState<BatchReview|null>(null);const [stale,setStale]=useState(false);
 async function load(){setItems(undefined);setSelected([]);setPreview(null);try{setItems(await api.pendingDescriptions());}catch(e){setError(asError(e));}}
 useEffect(()=>{void load();},[]);
 async function review(){if(!items)return;setBusy(true);setError(undefined);setStale(false);try{const result=await api.previewAcceptMany(selected);if(!matchesShown(items,result,selected)){setStale(true);await load();return;}setPreview(result);}catch(e){setError(asError(e));}finally{setBusy(false);}}
 async function accept(){if(!preview)return;setBusy(true);setError(undefined);try{const result=await api.acceptMany(preview.items.map(item=>item.id),preview.preview_token);setPreview(null);if(result.committed)onSaved();onClose();}catch(e){const failure=asError(e);setPreview(null);if(failure.code==="REVIEW_PREVIEW_CHANGED"){setStale(true);await load();}else setError(failure);}finally{setBusy(false);}}
 const eligible=items?selectable(items):[];
 return <Dialog open onOpenChange={open=>{if(!open&&!busy)onClose();}}><DialogContent closeLabel={t.close} className="batch-review-dialog" data-testid="batch-review"><DialogHeader><DialogTitle>{w.title}</DialogTitle><DialogDescription>{w.body}</DialogDescription></DialogHeader>
  {error&&<ErrorNotice error={error} t={t}/>}
  {stale&&<p role="status" data-testid="batch-review-stale">{w.changed}</p>}
  {!items?<p>{t.loading}</p>:!items.length?<EmptyNotice title={w.empty}/>:<>
   <div className="button-row"><Button size="sm" variant="ghost" data-testid="batch-select-all" disabled={busy} onClick={()=>setSelected(eligible)}>{w.all}</Button><Button size="sm" variant="ghost" data-testid="batch-select-none" disabled={busy||!selected.length} onClick={()=>setSelected([])}>{w.none}</Button></div>
   <div className="batch-review-list">{items.map(item=><article key={item.id} className="batch-review-item" data-testid={`batch-item-${item.id}`} data-blocked={item.source_changed}>
    <label className="checkbox-label"><Checkbox data-testid={`batch-select-${item.id}`} disabled={busy||item.source_changed} checked={selected.includes(item.id)} onCheckedChange={on=>setSelected(old=>toggle(old,item.id,on===true))}/><strong dir="auto">{visibleName(item.name)}</strong></label>
    {item.source_changed&&<p role="note">{w.alone} {reasonText("SOURCE_CHANGED_WITH_PENDING_PROPOSAL",t.locale)}</p>}
    <DescriptionReview original={item.current} proposed={item.proposed} t={t}/>
   </article>)}</div></>}
  <DialogFooter><Button variant="outline" disabled={busy} onClick={onClose}>{t.cancel}</Button><Button data-testid="batch-accept" disabled={busy||!selected.length} onClick={()=>void review()}><Check data-icon="inline-start"/>{w.accept(selected.length)}</Button></DialogFooter>
 </DialogContent>
 <Dialog open={preview!==null} onOpenChange={open=>{if(!open&&!busy)setPreview(null);}}><DialogContent closeLabel={t.close}><DialogHeader><DialogTitle>{w.confirm(preview?.items.length??0)}</DialogTitle><DialogDescription>{w.confirmBody}</DialogDescription></DialogHeader><ul className="batch-review-confirm">{preview?.items.map(item=><li key={item.id}><bdi>{item.id}</bdi></li>)}</ul><DialogFooter><Button variant="outline" disabled={busy} onClick={()=>setPreview(null)}>{t.cancel}</Button><Button data-testid="batch-confirm" disabled={busy} onClick={()=>void accept()}>{t.accept}</Button></DialogFooter></DialogContent></Dialog>
 </Dialog>;
}
