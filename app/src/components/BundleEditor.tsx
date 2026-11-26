import {visibleName} from "../lib/paths";
import { loadBundleSkills } from "../lib/bundle-skills";
import { skillCount } from "../lib/presentation";
import { useEffect, useState } from "react";
import { api, emptyFilter, asError, type Bundle, type CoreError, type Skill } from "../bridge";
import type { Text } from "../i18n";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Checkbox } from "./ui/checkbox";
import { Field, FieldGroup, FieldLabel } from "./ui/field";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { Choice, ErrorNotice } from "./common";
export function BundleEditor({t,initial,vault,onClose,onSaved}:{t:Text;initial:Bundle|null;vault:string;onClose:()=>void;onSaved:()=>void}) {
 const [skills,setSkills]=useState<Skill[]>(),[loadError,setLoadError]=useState<CoreError>(),[attempt,setAttempt]=useState(0);
 useEffect(()=>{
   let active=true;
   setSkills(undefined);setLoadError(undefined);
   void loadBundleSkills(vault,()=>api.catalog(emptyFilter)).then(items=>{if(active)setSkills(items);})
     .catch(e=>{if(active)setLoadError(asError(e));});
   return ()=>{active=false;};
 },[vault,attempt]);
 const editing=!!initial?.name;
 const [bundle,setBundle]=useState<Bundle>(initial||{name:"",title:"",agents:["claude","codex"],skills:[]});
 const [error,setError]=useState<CoreError>();const [busy,setBusy]=useState(false);const [preview,setPreview]=useState(false);
 function update(value:Partial<Bundle>){setBundle({...bundle,...value});setPreview(false);}
 function member(id:string,value:Partial<Bundle["skills"][number]>){update({skills:bundle.skills.map(s=>s.id===id?{...s,...value}:s)});}
 async function save(){setBusy(true);setError(undefined);try{const r=await api.saveBundle(bundle,editing,!preview);if(preview){if(r.committed)onSaved();onClose();}else{setPreview(true);if(r.warnings.length)setError({code:"BUNDLE_WARNING",message:r.warnings.join("\n")});}}catch(e){setError(asError(e));}finally{setBusy(false);}}
 return <Dialog open onOpenChange={v=>{if(!v&&!busy)onClose();}}><DialogContent className="bundle-dialog" closeLabel={t.close}><DialogHeader><DialogTitle>{editing?t.editBundle:t.newBundle}</DialogTitle><DialogDescription>{t.bundleSkills}</DialogDescription></DialogHeader>{error&&<ErrorNotice t={t} error={error}/>}
 <FieldGroup><Field><FieldLabel htmlFor="bundle-id">{t.bundleName}</FieldLabel><Input id="bundle-id" data-testid="bundle-id" value={bundle.name} disabled={editing||busy} onChange={e=>update({name:e.target.value})} dir="ltr"/></Field><Field><FieldLabel htmlFor="bundle-title">{t.bundleTitle}</FieldLabel><Input id="bundle-title" data-testid="bundle-title" value={bundle.title} disabled={busy} onChange={e=>update({title:e.target.value})} dir="auto"/></Field></FieldGroup>
 <div className="button-row">{["claude","codex","cursor"].map(agent=><label className="checkbox-label" key={agent}><Checkbox checked={bundle.agents.includes(agent)} onCheckedChange={v=>update({agents:v?[...bundle.agents,agent]:bundle.agents.filter(a=>a!==agent)})}/><bdi>{agent}</bdi></label>)}</div>
 {loadError?<><ErrorNotice error={loadError} t={t}/><Button variant="outline" onClick={()=>setAttempt(n=>n+1)}>{t.retry}</Button></>:!skills&&<p role="status">{t.loading}</p>}
 <div className="bundle-members">{skills?.map(skill=>{const selected=bundle.skills.find(s=>s.id===skill.id);return <div key={skill.id} className="bundle-member"><label className="checkbox-label"><Checkbox checked={!!selected} disabled={busy} aria-label={skill.id} onCheckedChange={v=>update({skills:v?[...bundle.skills,{id:skill.id,role:"primary"}]:bundle.skills.filter(s=>s.id!==skill.id)})}/><bdi>{visibleName(skill.name)}</bdi>{(selected?.audit_level??skill.audit_level)==="risky"&&<span className="notice">{t.risky}</span>}</label>{selected&&<div className="member-options"><Choice label={t.role} value={selected.role} options={[{value:"primary",label:t.primary},{value:"secondary",label:t.secondary}]} onChange={role=>member(skill.id,{role})}/><Input aria-label={t.groupName} placeholder={t.groupName} value={selected.group||""} onChange={e=>member(skill.id,{group:e.target.value||undefined})}/></div>}</div>;})}</div>
 {preview&&<p className="notice">{t.bundlePreview}: <bdi>{bundle.name}</bdi> · {skillCount(bundle.skills.length,t.locale)}</p>}
 <DialogFooter><Button variant="outline" onClick={onClose} disabled={busy}>{t.cancel}</Button><Button data-testid="bundle-save" onClick={()=>void save()} disabled={busy||!skills}>{preview?t.confirmSave:t.bundlePreview}</Button></DialogFooter></DialogContent></Dialog>;
}
