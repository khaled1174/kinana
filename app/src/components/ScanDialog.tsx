import {visibleName} from "../lib/paths";
import {useEffect,useMemo,useRef,useState} from "react";
import {settingsApi,asError,type CoreError,type ImportReport,type Audit} from "../bridge";
import type {Text} from "../i18n";
import {settingsMessages} from "../settings-i18n";
import {planScan,scanSelection,emptyChoice,scanFolder,selectScanFolder,type ScanChoice,type ScanItem} from "../lib/scan-plan";
import {Textarea} from "./ui/textarea";
import {Button} from "./ui/button";
import {Checkbox} from "./ui/checkbox";
import {Badge} from "./ui/badge";
import {Progress} from "./ui/progress";
import {Dialog,DialogContent,DialogHeader,DialogTitle,DialogDescription,DialogFooter} from "./ui/dialog";
import {ErrorNotice} from "./common";
import {CodeNotices} from "./CodeNotices";
import {PathView} from "./PathView";
import {AuditDetails} from "./AuditDetails";

// What the user chose in a scan review outlives the dialog: closing it and
// scanning the same folders again returns to the same list and choices.
const remembered=new Map<string,ScanChoice>();

/** Audit notes fold per skill: a summary with their count, the notes only when opened. */
function AuditFold({audit,name,t}:{audit:Audit;name:string;t:Text}) {
 const p=settingsMessages[t.locale as "ar"|"en"];const [open,setOpen]=useState(false);
 const level=audit.level in t?t[audit.level as keyof Text] as string:audit.level;
 if(audit.rules_version&&!audit.details.length)return <p className="scan-audit-clean">{level}</p>;
 return <details className="scan-audit" data-testid={`scan-audit-${name}`} onToggle={e=>setOpen((e.target as HTMLDetailsElement).open)}><summary>{p.scanAuditNotes}: {new Intl.NumberFormat(t.locale).format(audit.details.length)} · {level}</summary>{open&&<AuditDetails audit={audit} t={t}/>}</details>;
}

export function ScanDialog({t,roots,onClose,onImported,onScanned}:{t:Text;roots:string[];onClose:()=>void;onImported:(committed:boolean,ids:string[])=>Promise<void>;onScanned:()=>void}) {
 const p=settingsMessages[t.locale as "ar"|"en"],number=new Intl.NumberFormat(t.locale);
 const key=roots.join("\0");
 const [report,setReport]=useState<ImportReport>(),[choice,setChoice]=useState<ScanChoice>(()=>remembered.get(key)??emptyChoice()),[allowRisky,setAllowRisky]=useState(false);
 const [error,setError]=useState<CoreError>(),[cancelled,setCancelled]=useState(false),[running,setRunning]=useState(false),[starting,setStarting]=useState(true);
 const [importing,setImporting]=useState(false),[stage,setStage]=useState(""),[counts,setCounts]=useState({completed:0,total:0}),[cancelling,setCancelling]=useState(false),[generation,setGeneration]=useState(0);
 const started=useRef(""),importingNow=useRef(false),scannedCallback=useRef(onScanned),importedCallback=useRef(onImported),actions=useRef<HTMLDivElement>(null);
 scannedCallback.current=onScanned;importedCallback.current=onImported;
 const plan=useMemo(()=>report&&planScan(report),[report]);
 const selection=useMemo(()=>plan&&scanSelection(plan,choice),[plan,choice]);
 const [edited,setEdited]=useState<Record<string,string>>({});
 const [approval,setApproval]=useState<{key:string;token:string}>();
 const [descriptionError,setDescriptionError]=useState<CoreError>();
 const selectedPaths=selection?.select,primaryPaths=selection?.primary;
 const descriptions:Record<string,string>={};
 if(plan&&selectedPaths&&primaryPaths){
  for(const item of plan.fresh)if(selectedPaths.includes(item.path))descriptions[item.path]=edited[item.path]??item.description??"";
  for(const conflict of plan.conflicts)for(const version of conflict.versions)for(const item of version.items)
   if(primaryPaths.includes(item.path))descriptions[item.path]=edited[item.path]??item.description??"";
 }
 const approvalKey=selectedPaths?.length?JSON.stringify({selectedPaths,primaryPaths,allowRisky,descriptions}):"";
 useEffect(()=>{
  if(!report||!approvalKey){setApproval(undefined);setDescriptionError(undefined);return;}
  let active=true;
  setDescriptionError(undefined);
  const timer=setTimeout(()=>{
   const input=JSON.parse(approvalKey) as {selectedPaths:string[];primaryPaths:string[];allowRisky:boolean;descriptions:Record<string,string>};
   void settingsApi.previewScanSelection(report,input.selectedPaths,input.allowRisky,input.primaryPaths,input.descriptions)
    .then(token=>{if(active)setApproval({key:approvalKey,token});})
    .catch(error=>{if(active)setDescriptionError(asError(error));});
  },180);
  return()=>{active=false;clearTimeout(timer);};
 },[report,approvalKey]);
 const folders=plan?[...new Set([...plan.fresh,...plan.drifted,...plan.conflicts.flatMap(conflict=>conflict.versions.flatMap(version=>version.items))].map(scanFolder))].sort():[];
 const [chosenFolder,setChosenFolder]=useState<string>();
 const ar=t.locale==="ar";
 const resultsTitle=(index:number,title:string)=><h3 className="scan-results-heading"><bdi>{number.format(index)}.</bdi> {title}</h3>;
 const committing=stage==="committing",busy=running||starting;
 useEffect(()=>{remembered.set(key,choice);setAllowRisky(false);},[key,choice]);

 // Start, or on reopening reuse, the scan of these folders. The first open
 // and "scan again" each start exactly once, also under a double effect.
 useEffect(()=>{const run=`${key}\u0001${generation}`;if(started.current===run)return;started.current=run;let active=true;
  setStarting(true);setError(undefined);setCancelled(false);
  settingsApi.scan(roots,generation===0).then(()=>{if(active){setStarting(false);setRunning(true);}}).catch(e=>{if(active){setError(asError(e));setStarting(false);}});
  return()=>{active=false;started.current="";};
 },[key,generation]);// eslint-disable-line react-hooks/exhaustive-deps

 useEffect(()=>{if(!running)return;let active=true;let timer:ReturnType<typeof setTimeout>;
  async function poll(){try{const result=await settingsApi.scanStatus();if(!active)return;setStage(result.stage);setCounts({completed:result.completed,total:result.total});
   if(!result.done){timer=setTimeout(poll,80);return;}
   setRunning(false);setCancelling(false);setStage("");
   const wasImport=importingNow.current;importingNow.current=false;setImporting(false);
   if(result.error){if(result.error.code==="CANCELLED"&&wasImport)setCancelled(true);else setError(result.error);}
   if(result.report){setAllowRisky(false);setReport(result.report);if(result.imported){await importedCallback.current(result.report.committed,result.report.imported);return;}if(!wasImport)scannedCallback.current();}
  }catch(e){if(active){setError(asError(e));setRunning(false);setImporting(false);importingNow.current=false;}}}
  void poll();return()=>{active=false;clearTimeout(timer);};
 },[running]);// eslint-disable-line react-hooks/exhaustive-deps

 // An error or a cancellation is shown beside the buttons, where the user is.
 useEffect(()=>{if(error||cancelled)actions.current?.scrollIntoView({block:"nearest"});},[error,cancelled]);

 async function close(){if(importing||starting)return;if(running&&!await settingsApi.cancelScan())return;onClose();}
 async function cancelImport(){setCancelling(true);if(!await settingsApi.cancelScan())setCancelling(false);}
 function rescan(){setEdited({});setChosenFolder(undefined);remembered.delete(key);setChoice(emptyChoice());setAllowRisky(false);setReport(undefined);setGeneration(g=>g+1);}
 async function apply(){if(!selection||!report||!approval||approval.key!==approvalKey)return;setError(undefined);setCancelled(false);
  try{await settingsApi.commitScan(report,selection.select,allowRisky,selection.primary,descriptions,approval.token);importingNow.current=true;setImporting(true);setStage("scanning");setCounts({completed:0,total:0});setRunning(true);}catch(e){setError(asError(e));}}
 const toggle=(list:"unchecked"|"drift",path:string,on:boolean)=>setChoice(old=>({...old,[list]:on?[...old[list],path]:old[list].filter(item=>item!==path)}));

 const stageText=committing?p.scanSaving:cancelling?p.scanCancelling:stage==="staging"?p.scanStaging:stage==="checking"?p.scanChecking:importing?p.scanAdding:p.scanning;
 const counted=importing&&counts.total>0&&(stage==="staging"||stage==="checking");
 const descriptionInput=(item:ScanItem)=><label className="settings-label">{ar?"الوصف الذي سيُعتمد":"Description to approve"}
  <Textarea rows={2} dir="auto" data-testid={`scan-description-${item.name}`} value={edited[item.path]??item.description??""} disabled={busy}
   onChange={event=>{const value=event.target.value;setEdited(previous=>({...previous,[item.path]:value}));}}/>
 </label>;
 const row=(item:ScanItem,control:React.ReactNode,canEdit=false)=><article key={item.path}><label className="checkbox-label">{control}<strong dir="auto">{visibleName(item.name||item.id||t.broken)}</strong>{item.id&&item.id!==item.name&&<code>{visibleName(item.id)}</code>}<Badge variant="outline">{item.status in p?p[item.status as "new"]:item.status}</Badge></label><PathView path={item.path} tail/><AuditFold audit={item.audit} name={item.name??item.path} t={t}/>{canEdit&&descriptionInput(item)}</article>;

 return <Dialog open onOpenChange={value=>{if(!value)void close();}}><DialogContent className="scan-dialog" closeLabel={t.close} showCloseButton={!importing&&!starting}>
  <DialogHeader><DialogTitle>{p.scanTitle}</DialogTitle><DialogDescription>{p.scanHint}</DialogDescription></DialogHeader>
  <div className="scan-roots">{roots.map(root=><PathView key={root} path={root}/>)}</div>
  {report&&plan&&<>
   <div className="scan-counts" data-testid="scan-counts">{(["new","duplicate","variant","broken"] as const).map(state=><span key={state}>{p[state]}: {number.format(report.items.filter(i=>i.status===state).length)}</span>)}</div>
   {folders.length>0&&<div className="scan-folder-choices" aria-label={ar?"اختيار نسخ المجلد":"Select folder copies"}>
    {folders.map(folder=><div key={folder}><PathView path={folder} tail/><Button size="sm" variant="outline" data-testid="scan-select-folder" disabled={busy} onClick={()=>{if(plan)setChoice(old=>selectScanFolder(plan,old,folder));setChosenFolder(folder);}}>{ar?"اختر كل النسخ من هذا المجلد":"Select all copies from this folder"}</Button></div>)}
    {chosenFolder&&plan.conflicts.some(conflict=>!choice.primary[conflict.name]&&conflict.versions.some(version=>version.items.some(item=>scanFolder(item)===chosenFolder)))&&<p role="status" data-testid="scan-folder-conflicts">{p.scanConflictHint}</p>}
   </div>}
   <div className="scan-results" data-testid="scan-results">{report.items.length?<>
    {plan.fresh.length>0&&<section className="scan-section" data-testid="scan-new">{resultsTitle(1,`${p.scanNewTitle} (${number.format(plan.fresh.length)})`)}{plan.fresh.map(item=>row(item,<Checkbox data-testid={`scan-select-${item.name}`} checked={!choice.unchecked.includes(item.path)} disabled={busy} onCheckedChange={checked=>toggle("unchecked",item.path,!checked)}/>,!choice.unchecked.includes(item.path)))}</section>}
    {plan.conflicts.length+plan.drifted.length+plan.duplicates.length>0&&<div data-testid="scan-existing-results">
    {resultsTitle(2,ar?"النسخ المتكررة والمختلفة":"Duplicates and different versions")}
    {plan.conflicts.length>0&&<section className="scan-section" data-testid="scan-conflicts"><h4>{p.scanConflictTitle} ({number.format(plan.conflicts.length)})</h4><p className="settings-help">{p.scanConflictHint}</p>{plan.conflicts.map(conflict=><fieldset key={conflict.name} className="scan-conflict" data-testid={`scan-conflict-${conflict.name}`} disabled={busy}><legend dir="auto">{visibleName(conflict.name)}</legend>
     {conflict.versions.map((version,index)=>{const first=version.items[0];return <div key={version.hash}><label className="scan-version"><input type="radio" name={`primary-${conflict.name}`} data-testid={`scan-primary-${conflict.name}-${index}`} checked={choice.primary[conflict.name]===first.path} onChange={()=>setChoice(old=>({...old,primary:{...old.primary,[conflict.name]:first.path}}))}/><span><PathView path={first.path} tail/>{version.items.length>1&&<span className="scan-version-more">{p.scanSameContent}: {number.format(version.items.length-1)}</span>}<AuditFold audit={first.audit} name={`${conflict.name}-${index}`} t={t}/></span></label>{choice.primary[conflict.name]===first.path&&descriptionInput(first)}</div>;})}
     <label className="scan-version"><input type="radio" name={`primary-${conflict.name}`} data-testid={`scan-primary-${conflict.name}-none`} checked={!choice.primary[conflict.name]} onChange={()=>setChoice(old=>{const primary={...old.primary};delete primary[conflict.name];return {...old,primary};})}/><span>{p.scanSkipName}</span></label>
    </fieldset>)}</section>}
    {plan.drifted.length>0&&<section className="scan-section" data-testid="scan-drifted"><h4>{p.scanDriftTitle} ({number.format(plan.drifted.length)})</h4><p className="settings-help">{p.scanDriftHint}</p>{plan.drifted.map(item=>row(item,<Checkbox data-testid={`scan-drift-${item.name}`} checked={choice.drift.includes(item.path)} disabled={busy} onCheckedChange={checked=>toggle("drift",item.path,!!checked)}/>))}</section>}
    {plan.duplicates.length>0&&<details className="scan-section scan-fold" data-testid="scan-duplicates"><summary>{p.scanDuplicatesTitle} ({number.format(plan.duplicates.length)})</summary><p className="settings-help">{p.scanDuplicateHint}</p>{plan.duplicates.map(item=><article key={item.path}><strong dir="auto">{visibleName(item.name??"")}</strong>{item.id&&item.id!==item.name&&<code>{visibleName(item.id)}</code>}<PathView path={item.path} tail/></article>)}</details>}
    </div>}
    {plan.broken.length>0&&<details className="scan-section scan-fold" data-testid="scan-broken"><summary><bdi>{number.format(3)}.</bdi> {p.scanBrokenTitle} ({number.format(plan.broken.length)})</summary>{plan.broken.map(item=><article key={item.path}><PathView path={item.path} tail/>{item.error&&<p className="notice" dir="auto">{visibleName(item.error)}</p>}</article>)}</details>}
   </>:<p>{p.empty}</p>}</div>
   <CodeNotices codes={report.diagnostic_codes} locale={t.locale}/>
  </>}
  <div className="scan-actions" ref={actions} data-testid="scan-actions">
   {error&&<div data-testid="scan-error"><ErrorNotice error={error} t={t}/>{error.code==="NAME_CONFLICT"&&plan&&plan.conflicts.length>0&&<p className="settings-help" dir="auto">{p.scanConflictNames}: {plan.conflicts.map(c=>visibleName(c.name)).join("، ")}</p>}<details className="scan-error-details"><summary>{p.scanErrorDetails}</summary><p dir="auto">{visibleName(error.message)}</p></details></div>}
   {descriptionError&&<ErrorNotice error={descriptionError} t={t}/>}
   {cancelled&&<p className="scan-cancelled" role="status" data-testid="scan-cancelled">{p.scanCancelled}</p>}
   {busy&&<div role="status" className="operation-progress" data-testid="scan-progress"><p>{stageText}{counted&&<> · <bdi data-testid="scan-progress-count">{number.format(counts.completed)} {p.scanOf} {number.format(counts.total)}</bdi></>}</p><Progress value={counted?Math.round(counts.completed*100/counts.total):null} aria-label={t.progress}/></div>}
   {report&&selection&&!busy&&<p className="scan-summary" data-testid="scan-summary">{p.scanToAdd}: {number.format(selection.added)} · {p.scanToRecord}: {number.format(selection.locations)}</p>}
   {report&&selection&&selection.risky>0&&!allowRisky&&<p className="scan-risky-hint" data-testid="scan-risky-hint">{p.scanRiskyHint} ({number.format(selection.risky)}). {p.scanRiskyAction}</p>}
   {report&&<label className="checkbox-label"><Checkbox data-testid="scan-allow-risky" checked={allowRisky} onCheckedChange={v=>setAllowRisky(!!v)} disabled={busy}/>{p.approveRisk}</label>}
   <DialogFooter>
    {importing&&running?<Button variant="outline" data-testid="cancel-scan-import" disabled={committing||cancelling} onClick={()=>void cancelImport()}>{p.scanCancelImport}</Button>:<Button variant="outline" data-testid="cancel-scan" disabled={starting} onClick={()=>void close()}>{p.scanClose}</Button>}
    {!busy&&<Button variant="ghost" data-testid="rescan" onClick={rescan}>{p.scanRescan}</Button>}
    <Button data-testid="confirm-scan-import" disabled={busy||!report||approval?.key!==approvalKey||!selection?.select.length||(!!selection&&selection.risky>0&&!allowRisky)} onClick={()=>void apply()}>{importing&&running?p.scanAdding:`${p.import} (${number.format(selection?.added??0)})`}</Button>
   </DialogFooter>
  </div>
 </DialogContent></Dialog>;
}
