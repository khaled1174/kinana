import {useEffect,useRef,useState} from "react";
import {sourceApi,asError,type CoreError,type SourceUpdateOptions,type SourceUpdateReport,type SourceFileInfo} from "../bridge";
import type {Text} from "../i18n";
import {sourceMessages} from "../source-i18n";
import {reasonText} from "../reasons";
import {PathView} from "./PathView";
import {AuditDetails} from "./AuditDetails";
import {Button} from "./ui/button";
import {Input} from "./ui/input";
import {Checkbox} from "./ui/checkbox";
import {Progress} from "./ui/progress";
import {Dialog,DialogContent,DialogHeader,DialogTitle,DialogDescription,DialogFooter} from "./ui/dialog";
import {ErrorNotice} from "./common";

export function SourceUpdateDialog({id,t,onClose,onSaved}:{id:string;t:Text;onClose:()=>void;onSaved:()=>void}){
 const p=sourceMessages[t.locale as "ar"|"en"];
 const [reference,setReference]=useState("");const [report,setReport]=useState<SourceUpdateReport|null>(null);
 const [error,setError]=useState<CoreError|null>(null),[busy,setBusy]=useState(false),[committing,setCommitting]=useState(false),[allowRisky,setAllowRisky]=useState(false);
 const sourceId=useRef(""),active=useRef(true),cancelled=useRef(false),started=useRef(false);
 useEffect(()=>{active.current=true;if(!started.current){started.current=true;void run({id,dry_run:true});}return()=>{active.current=false;};},[id]);
 async function run(options:SourceUpdateOptions){setBusy(true);setError(null);cancelled.current=false;const job=crypto.randomUUID();sourceId.current=job;try{
  await sourceApi.start(job,{action:"update",options});
  while(active.current){const state=await sourceApi.status(job);if(!active.current||cancelled.current)return;setCommitting(state.committing);if(state.done){if(cancelled.current)return;if(state.error)throw state.error;const result=state.report as SourceUpdateReport;setReport(result);if(result.committed){onSaved();onClose();}return;}await new Promise(resolve=>setTimeout(resolve,100));}
 }catch(e){if(active.current&&!cancelled.current){setError(asError(e));setReport(null);}}finally{try{await sourceApi.cancel(job);}catch(e){if(active.current)setError(asError(e));}if(active.current){setBusy(false);setCommitting(false);}}}
 async function close(){if(committing)return;cancelled.current=true;try{const accepted=await sourceApi.cancel(sourceId.current);if(accepted)onClose();}catch(e){setError(asError(e));}}
 function preview(){setAllowRisky(false);setReport(null);void run({id,reference:reference||undefined,dry_run:true});}
 function confirm(){if(report)void run({id,reference:reference||undefined,confirm:report.preview_token,allow_risky:allowRisky});}
 function details(value:SourceFileInfo|null){return value?<dl className="source-file-info"><dt>{value.kind==="directory"?t.locale==="ar"?"مجلد":"Directory":t.locale==="ar"?"ملف":"File"}</dt><dd><bdi dir="ltr">{p.mode}: {value.mode.toString(8)} · {value.bytes} {p.bytes}</bdi></dd>{value.content_hash&&<dd><bdi dir="ltr">{value.content_hash}</bdi></dd>}</dl>:<p>—</p>;}
 return <Dialog open onOpenChange={open=>{if(!open)void close();}}><DialogContent className="source-update-dialog" closeLabel={t.close} showCloseButton={!busy}><DialogHeader><DialogTitle>{p.review} · <bdi>{id}</bdi></DialogTitle><DialogDescription>{p.intro}</DialogDescription></DialogHeader>
  {error&&<ErrorNotice error={error} t={t}/>}
  <div className="source-reference"><label>{p.reference}<Input data-testid="source-reference" value={reference} onChange={e=>{setReference(e.target.value);setReport(null);setAllowRisky(false);}} placeholder={p.referenceHint} disabled={busy} dir="ltr"/></label><Button variant="outline" data-testid="preview-source-update" disabled={busy} onClick={preview}>{p.preview}</Button></div>
  {busy&&<div role="status"><p>{committing?p.committing:p.fetching}</p><Progress aria-label={t.progress}/></div>}
  {report&&<div className="source-update-preview" data-testid="source-update-preview"><PathView path={report.repository}/><p><bdi dir="ltr">{report.tracking_kind} / {report.reference}</bdi></p><dl><dt>{p.oldCommit}</dt><dd><bdi dir="ltr">{report.old_commit}</bdi></dd><dt>{p.newCommit}</dt><dd><bdi dir="ltr">{report.new_commit}</bdi></dd></dl>
   {report.warnings.map((warning,i)=><section className="source-warning" key={i} data-testid="source-update-warning"><strong>{reasonText(warning.reason_code,t.locale)}</strong>{warning.old_description!==null&&<><h4>{p.old}</h4><p dir="auto">{warning.old_description}</p><h4>{p.new}</h4><p dir="auto">{warning.new_description}</p></>}</section>)}
   {!report.changed&&<p>{p.noChange}</p>}
   <p>{p.limit}: {report.diff_byte_limit.toLocaleString(t.locale)} · {p.omitted}: {report.omitted_files.toLocaleString(t.locale)}</p>
   <section className="source-diff-files">{report.files.map(file=><article key={file.path}><h3><PathView path={file.path}/></h3><div className="source-file-pair"><div><h4>{p.old}</h4>{details(file.old)}</div><div><h4>{p.new}</h4>{details(file.new)}</div></div>{file.unified_diff!==null?<pre dir="ltr" data-testid="source-diff">{file.unified_diff}</pre>:<p className="notice">{file.truncated?p.omitted:p.binary}</p>}</article>)}</section>
   <section><h3>{p.oldAudit} · {report.old_audit.level in t?t[report.old_audit.level as keyof Text]:t.unscanned}</h3><AuditDetails audit={report.old_audit} t={t}/></section><section><h3>{p.newAudit} · {report.new_audit.level in t?t[report.new_audit.level as keyof Text]:t.unscanned}</h3><AuditDetails audit={report.new_audit} t={t}/></section>
   {report.new_audit.level==="risky"&&<label className="source-risk-consent"><Checkbox data-testid="allow-source-risky" disabled={busy} checked={allowRisky} onCheckedChange={checked=>setAllowRisky(checked===true)}/>{p.allowRisky}</label>}
  </div>}
  <DialogFooter><Button variant="outline" data-testid="cancel-source-update" disabled={committing} onClick={()=>void close()}>{t.cancel}</Button><Button data-testid="confirm-source-update" disabled={busy||!report?.changed||(report.new_audit.level==="risky"&&!allowRisky)} onClick={confirm}>{p.confirm}</Button></DialogFooter>
 </DialogContent></Dialog>;
}
