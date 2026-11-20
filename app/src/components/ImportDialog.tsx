import {visibleName} from "../lib/paths";
import {AuditDetails} from "./AuditDetails";
import {PathView} from "./PathView";
import {CodeNotices} from "./CodeNotices";
import {reasonText} from "../reasons";
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, FileArchive } from "lucide-react";
import { api, asError, type CoreError, type ImportReport } from "../bridge";
import type { Text } from "../i18n";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Checkbox } from "./ui/checkbox";
import { Progress } from "./ui/progress";
import { Field, FieldGroup, FieldLabel } from "./ui/field";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { ErrorNotice, labelFor } from "./common";

export function ImportDialog({t,initial,onClose,onSaved}:{t:Text;initial:string;onClose:()=>void;onSaved:()=>void}) {
  const [input,setInput]=useState(initial);const [report,setReport]=useState<ImportReport>();const [error,setError]=useState<CoreError>();
  const [busy,setBusy]=useState(false);const [poll,setPoll]=useState(false);const [stage,setStage]=useState("preparing");
  const [prepared,setPrepared]=useState(false);const [committing,setCommitting]=useState(false);const [selected,setSelected]=useState<string[]>([]);
  const [risky,setRisky]=useState(false);const [suffix,setSuffix]=useState("");
  function show(value:ImportReport){setRisky(false);setReport(value);setSelected(value.items.filter(i=>i.selected).map(i=>i.path));}
  async function inspect(){setBusy(true);setError(undefined);try{await api.cancelImport();show(await api.previewImport(input));setPrepared(false);}catch(e){setError(asError(e));}finally{setBusy(false);}}
  async function prepare(){setBusy(true);setError(undefined);try{await api.prepareImport(input);setStage("preparing");setPoll(true);}catch(e){setError(asError(e));setBusy(false);}}
  async function cancel(close=true){try{if(await api.cancelImport()){setPoll(false);setBusy(false);setPrepared(false);setCommitting(false);if(close)onClose();}}catch(e){setError(asError(e));}}
  useEffect(()=>{if(!poll)return;let active=true;let timer:ReturnType<typeof setTimeout>;
    async function tick(){try{const status=await api.importStatus();if(!active)return;setStage(status.stage);if(status.done){setPoll(false);setBusy(false);if(status.error){setError(status.error);setPrepared(status.prepared);setCommitting(false);if(status.error.code==="IMPORT_PREVIEW_CHANGED"){setRisky(false);setPrepared(false);setReport(undefined);await api.cancelImport();}else if(!status.prepared)await api.cancelImport();}else if(status.report){if(committing){await api.cancelImport();if(status.report.committed)onSaved();onClose();}else{show(status.report);setPrepared(true);}}}else timer=setTimeout(tick,150);}catch(e){if(active){setError(asError(e));setPoll(false);setBusy(false);}}}
    void tick();return()=>{active=false;clearTimeout(timer);};
  },[poll,committing]);
  async function commit(){if(!report)return;setBusy(true);setError(undefined);try{await api.commitImport(report,selected,risky,suffix||null);setCommitting(true);setStage("staging");setPoll(true);}catch(e){setError(asError(e));setBusy(false);}}
  async function pick(directory:boolean){try{const path=await open({directory,multiple:false,...(!directory?{filters:[{name:"ZIP",extensions:["zip"]}]}:{})});if(typeof path==="string"){setInput(path);setReport(undefined);}}catch(e){setError(asError(e));}}
  return <Dialog open onOpenChange={value=>{if(!value)void cancel();}}><DialogContent className="import-dialog" closeLabel={t.close} showCloseButton={!busy}><DialogHeader><DialogTitle>{t.importTitle}</DialogTitle><DialogDescription>{t.selectionNote}</DialogDescription></DialogHeader>
    {error&&<ErrorNotice error={error} t={t}/>}
    <FieldGroup><Field><FieldLabel htmlFor="import-source">{t.inputSource}</FieldLabel><Input id="import-source" data-testid="import-source" value={input} disabled={busy||prepared} onChange={e=>{setInput(e.target.value);setReport(undefined);}} dir="auto"/><div className="button-row"><Button variant="outline" size="sm" disabled={busy||prepared} onClick={()=>void pick(true)}><FolderOpen data-icon="inline-start"/>{t.chooseFolder}</Button><Button variant="outline" size="sm" disabled={busy||prepared} onClick={()=>void pick(false)}><FileArchive data-icon="inline-start"/>{t.chooseZip}</Button></div></Field></FieldGroup>
    {busy&&<div role="status" className="operation-progress"><p>{labelFor(t,stage)}</p><Progress aria-label={t.progress} value={null}/></div>}
    {report?.requires_preparation&&<p className="notice">{t.prepareNote}</p>}
    {report&&!report.requires_preparation&&<><p className="minor-label">{t.selectImport}</p><div className="import-items">{report.items.map(item=><div className="import-item" key={item.path}><label><Checkbox aria-label={visibleName(item.name||item.path)} checked={selected.includes(item.path)} disabled={busy||item.status==="broken"} onCheckedChange={checked=>{setRisky(false);setSelected(checked?[...selected,item.path]:selected.filter(id=>id!==item.path));}}/><strong dir="auto">{visibleName(item.name||item.path)}</strong>{item.id&&item.id!==item.name&&<code>{visibleName(item.id)}</code>}<span>{labelFor(t,item.status)}</span></label><p><PathView path={item.path}/></p><p>{labelFor(t,item.audit.level)}{item.error&&` · ${reasonText(item.reason_code||"IMPORT_ITEM_INVALID",t.locale)}`}</p><AuditDetails audit={item.audit} t={t}/></div>)}</div>
      <Field><FieldLabel htmlFor="import-suffix">{t.suffix}</FieldLabel><Input id="import-suffix" value={suffix} onChange={e=>setSuffix(e.target.value)} disabled={busy} dir="ltr"/></Field>
      <label className="checkbox-label"><Checkbox checked={risky} disabled={busy} onCheckedChange={v=>setRisky(v===true)}/>{t.allowRisky}</label>
    </>}
    <CodeNotices codes={report?.diagnostic_codes} locale={t.locale}/>
    <DialogFooter><Button variant="outline" disabled={busy&&stage==="committing"} onClick={()=>void cancel()}>{t.cancel}</Button>{!prepared ? <Button disabled={busy||!input.trim()} data-testid="import-inspect" onClick={()=>void(report?prepare():inspect())}>{report?t.prepare:t.inspect}</Button>:<Button disabled={busy||selected.length===0} data-testid="import-confirm" onClick={()=>void commit()}>{t.importConfirm}</Button>}</DialogFooter>
  </DialogContent></Dialog>;
}
