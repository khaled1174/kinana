import {visibleName} from "../lib/paths";
import {PathView} from "./PathView";
import {CodeNotices} from "./CodeNotices";
import {reasonText} from "../reasons";
import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, FolderKanban, RefreshCw, Plus, Package, Link2, Copy, Unlink, ArrowRightLeft, Search } from "lucide-react";
import { api, projectApi, asError, emptyFilter, type Catalog, type Bundle, type CoreError, type Project, type ProjectPreview, type ProjectRequest, type Skill } from "../bridge";
import type { Text } from "../i18n";
import { projectMessages } from "../project-i18n";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Badge } from "./ui/badge";
import { Checkbox } from "./ui/checkbox";
import { Progress } from "./ui/progress";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { Choice, EmptyNotice, ErrorNotice } from "./common";
import { ScanDialog } from "./ScanDialog";

type Operation = {action:"use"|"unuse"|"adopt";bundle?:boolean;id?:string;ids?:string[];agent?:string};
function within(path:string, parent:string) {
 const parts=(value:string)=>value.normalize("NFC").toLowerCase().split("/").filter(part=>part&&part!==".");
 const components=parts(path);
 return parts(parent).every((part,index)=>part===components[index]);
}

export function ProjectsScreen({t,home,onImported}:{t:Text;home:string;onImported:(committed:boolean)=>void}) {
 const p=projectMessages[t.locale as "ar"|"en"];
 const [paths,setPaths]=useState<string[]>([]),[selected,setSelected]=useState<string|null>(null),[projects,setProjects]=useState<Project[]>([]);
 const [query,setQuery]=useState(""),[loading,setLoading]=useState(false),[error,setError]=useState<CoreError|null>(null),[operation,setOperation]=useState<Operation|null>(null),[notice,setNotice]=useState(false);
 const [catalog,setCatalog]=useState<Catalog|null>(null);
 const [skipped,setSkipped]=useState<string[]>([]);
 const [scan,setScan]=useState<string[]|null>(null);
 const refreshSequence=useRef(0);
 const refresh=useCallback(async()=>{
  const sequence=++refreshSequence.current;
  setLoading(true);
  setError(null);
  try {
   const [report,vault]=await Promise.all([projectApi.list(paths),api.catalog(emptyFilter)]);
   if(sequence===refreshSequence.current){setProjects(report);setCatalog(vault);}
  } catch(e) {
   if(sequence===refreshSequence.current){setCatalog(null);setError(asError(e));}
  } finally {
   if(sequence===refreshSequence.current)setLoading(false);
  }
 },[paths]);
 useEffect(()=>{void refresh();},[refresh]);
 const project=projects.find(item=>item.path===selected);
 const skillHashes=new Map(catalog?.items.map(({skill})=>[skill.id,skill.content_hash]));
 const local=project?.local??[];
 const matching=local.filter(({entry})=>entry.status==="duplicate"&&entry.skill_id&&
  entry.content_hash&&skillHashes.get(entry.skill_id)===entry.content_hash);
 const unmanaged=local.filter(item=>!matching.includes(item));
 const importRoots=[...new Set(local.filter(({entry})=>entry.status!=="broken").map(({entry})=>entry.path))];
 const importAvailable=project&&catalog&&home&&
  !within(project.path,catalog.vault)&&!within(catalog.vault,project.path)&&
  !within(home,project.path)&&!within(project.path,home+"/.kinana");
 const count=importRoots.length.toLocaleString(t.locale);
 const canOfferImport=importAvailable&&importRoots.length>0&&!skipped.includes(project.path);
 function skipImport(path:string){setSkipped(old=>old.includes(path)?old:[...old,path]);}
 function localSkill({agent,entry}:Project["local"][number],skillId?:string|null){
  return <article key={`${agent}-${entry.path}`} className="local-row"
    data-testid={skillId?"project-linked-skill":undefined} data-skill-id={skillId??undefined}>
   <div>
    <strong dir="auto">{visibleName(entry.name||entry.path.split("/").at(-1)||"")}</strong>
    <Badge variant="outline">{agent==="claude"?t.claude:agent==="cursor"?t.cursor:t.codex}</Badge>
    {skillId?<Badge variant="secondary"><bdi>{visibleName(skillId)}</bdi></Badge>:
     <Badge variant="secondary">{entry.status in t?t[entry.status as keyof Text]:entry.status}</Badge>}
   </div>
   <p className="project-path"><PathView path={entry.path} project={project?.path}/></p>
   {entry.error&&<p dir="auto" className="notice">{reasonText("IMPORT_ITEM_INVALID",t.locale)}</p>}
  </article>;
 }
 async function choose(){try {const path=await open({directory:true,multiple:false,title:p.choose});if(typeof path==="string"){setPaths(old=>Array.from(new Set([...old,path])));setSelected(path);setNotice(false);}}catch(e){setError(asError(e));}}
 return <div className="projects-screen">
  <div className="projects-toolbar"><label className="project-search"><Search/><Input aria-label={p.search} placeholder={p.search} value={query} onChange={e=>setQuery(e.target.value)}/></label><Button variant="outline" data-testid="refresh-projects" disabled={loading} onClick={()=>void refresh()} aria-label={p.refresh} title={p.refresh}><RefreshCw/></Button><Button variant="outline" data-testid="choose-project" onClick={()=>void choose()}><FolderPlus/>{p.choose}</Button></div>
  {error&&<ErrorNotice error={error} t={t}/>}
  {notice&&<div className="project-success" role="status" data-testid="project-success"><strong>{p.done}</strong><p>{p.reload}</p></div>}
  <div className="projects-layout" aria-busy={loading}>
   <section className="project-list" aria-label={t.projects}>{loading&&!projects.length?<p className="notice">{t.loading}</p>:projects.length?projects.filter(item=>item.path.toLocaleLowerCase().includes(query.toLocaleLowerCase())).map(item=><button type="button" className="project-row" key={item.path} data-testid="project-row" data-path={item.path} data-selected={item.path===selected} onClick={()=>{setSelected(item.path);setNotice(false);}}><FolderKanban/><div><strong dir="auto">{visibleName((item.name)??"")}</strong><p className="project-path"><PathView path={item.path}/></p><span>{item.activations.length.toLocaleString(t.locale)} · {p.active}</span></div><Badge variant="outline">{item.state in p?p[item.state as keyof typeof p]:item.state}</Badge></button>):<EmptyNotice title={p.empty} body={p.emptyBody}><Button variant="outline" onClick={()=>void choose()}>{p.choose}</Button></EmptyNotice>}</section>
   <section className="project-details">{project?<>
    <div className="project-heading"><div><h2 dir="auto">{visibleName(project.name)}</h2><p className="project-path"><PathView path={project.path}/></p></div><Badge variant="secondary">{p.bundle}: <bdi>{project.bundle||p.noBundle}</bdi></Badge></div>
    <CodeNotices codes={project.diagnostic_codes} locale={t.locale}/>
    <div className="project-actions"><Button variant="outline" data-testid="activate-project" disabled={!['ok','no_lock'].includes(project.state)} onClick={()=>setOperation({action:"use"})}><Plus/>{p.activate}</Button><Button variant="outline" data-testid="apply-project-bundle" disabled={!['ok','no_lock'].includes(project.state)} onClick={()=>setOperation({action:"use",bundle:true})}><Package/>{p.applyBundle}</Button></div>
    {canOfferImport&&<section className="project-import-offer" aria-label={p.reviewLocal} data-testid="project-import-offer">
     <p role="status">{t.locale==="ar"?`وجدنا ${count} مهارة في هذا المشروع`:`Local skills found in this project: ${count}`}</p>
     <div className="button-row">
      <Button data-testid="review-project-skills" disabled={loading}
       onClick={()=>setScan(importRoots)}>{p.reviewLocal}</Button>
      <Button variant="ghost" data-testid="skip-project-skills" onClick={()=>skipImport(project.path)}>{p.skipLocal}</Button>
     </div>
    </section>}
    <div className="project-section-title"><h3>{p.active}</h3>{project.activations.length>0&&<Button variant="ghost" size="sm" data-testid="unuse-all" onClick={()=>setOperation({action:"unuse",ids:Array.from(new Set(project.activations.map(a=>a.id)))})}><Unlink/>{p.removeAll}</Button>}</div>
    <div className="activation-list" data-testid="project-activations">{project.activations.length?project.activations.map(a=><article className="activation-row" key={`${a.agent}-${a.id}`} data-testid={`activation-${a.agent}-${a.id}`} data-state={a.state}><div className="activation-top"><strong dir="auto">{visibleName(a.name)}</strong><Badge variant="outline">{a.agent==="claude"?t.claude:a.agent==="cursor"?t.cursor:t.codex}</Badge><span className={`project-state state-${a.state}`} title={reasonText(a.reason_code,t.locale)}>{a.state in p?p[a.state as keyof typeof p]:a.state}</span></div><div className="activation-meta"><span>{a.mode==="link"?<Link2/>:<Copy/>}{a.mode==="link"?p.link:p.copy} · {a.invocation==="manual"?p.manual:p.auto}</span><Button variant="ghost" size="sm" data-testid={`unuse-${a.agent}-${a.id}`} onClick={()=>setOperation({action:"unuse",id:a.id,agent:a.agent})}><Unlink/>{p.remove}</Button></div><p className="project-path"><PathView path={a.target} project={project.path}/></p>{a.agent==="cursor"&&<p className="project-help" data-testid="cursor-editor-support">{p.cursorSupport}</p>}{a.target.includes("/.agents/skills/")&&<p className="project-help" data-testid="shared-agent-path"><bdi dir="ltr">.agents/skills</bdi> · {p.sharedPath}</p>}{a.state!=="current"&&<p className="notice" dir="auto">{reasonText(a.reason_code,t.locale)}</p>}</article>):<p className="project-empty">{p.noActive}</p>}</div>
    {matching.length>0&&<section aria-label={p.linkedLocal} data-testid="project-linked-skills">
     <div className="project-section-title"><h3>{p.linkedLocal}</h3></div>
     <div>{matching.map(item=>localSkill(item,item.entry.skill_id))}</div>
    </section>}
    <div className="project-section-title"><h3>{p.local}</h3>{project.local.length>0&&<Button variant="outline" size="sm" data-testid="adopt-project" onClick={()=>setOperation({action:"adopt"})}><ArrowRightLeft/>{p.adopt}</Button>}</div>
    <p className="project-help">{p.localNote}</p>
    <div className="local-list">{unmanaged.length?unmanaged.map(item=>localSkill(item)):
     <p className="project-empty">{p.noLocal}</p>}</div>
   </>:<EmptyNotice title={p.select} body={p.selectBody}/>}</section>
  </div>
  {scan&&<ScanDialog t={t} roots={scan} onScanned={()=>{}} onClose={()=>setScan(null)}
   onImported={async committed=>{
    setScan(null);
    onImported(committed);
    await refresh();
   }}/>}
  {operation&&project&&<ProjectDialog t={t} project={project.path} operation={operation} onClose={()=>setOperation(null)} onSaved={async()=>{setNotice(true);await refresh();}}/>}
 </div>;
}
function ProjectDialog({t,project,operation,onClose,onSaved}:{t:Text;project:string;operation:Operation;onClose:()=>void;onSaved:()=>Promise<void>}) {
 const p=projectMessages[t.locale as "ar"|"en"];
 const [skills,setSkills]=useState<Skill[]>([]),[bundles,setBundles]=useState<Bundle[]>([]),[ids,setIds]=useState<string[]>(operation.id?[operation.id]:(operation.ids||[]));
 const [bundle,setBundle]=useState(""),[agent,setAgent]=useState(operation.agent||"claude"),[mode,setMode]=useState("link"),[secondary,setSecondary]=useState<string[]>([]);
 const [preview,setPreview]=useState<ProjectPreview|null>(null),[error,setError]=useState<CoreError|null>(null),[busy,setBusy]=useState(false),[running,setRunning]=useState(false),[committing,setCommitting]=useState(false);
 const mounted=useRef(true),cancelled=useRef(false);
 useEffect(()=>{mounted.current=true;void Promise.all([api.catalog({query:"",category:null,audit:null,agent:null,state:null,pinned:false}),api.bundles()]).then(([catalog,b])=>{if(mounted.current){setSkills(catalog.items.map(i=>i.skill));setBundles(b);}}).catch(e=>setError(asError(e)));return()=>{mounted.current=false;};},[]);
 const request:ProjectRequest=operation.action==="use"?{action:"use",project,selections:operation.bundle?[bundle]:ids,agent,mode,include_secondary:secondary}:operation.action==="unuse"?{action:"unuse",project,ids,agent:operation.agent||"all"}:{action:"adopt",project,agent};
 const title=operation.action==="unuse"?p.unuseTitle:operation.action==="adopt"?p.adoptTitle:operation.bundle?p.bundleTitle:p.activateTitle;
 async function inspect(){setBusy(true);setError(null);cancelled.current=false;try{const plan=await projectApi.preview(request);if(!cancelled.current&&mounted.current)setPreview(plan);}catch(e){if(mounted.current)setError(asError(e));}finally{if(mounted.current)setBusy(false);}}
 async function cancel(){if(running){if(await projectApi.cancel())cancelled.current=true;return;}cancelled.current=true;onClose();}
 async function apply(){if(!preview)return;setBusy(true);setError(null);cancelled.current=false;try{await projectApi.start(preview.request,preview.token);setRunning(true);while(mounted.current){const status=await projectApi.status();setCommitting(status.committing);if(status.done){await projectApi.cancel();if(cancelled.current){onClose();return;}if(status.error)throw status.error;await onSaved();onClose();return;}await new Promise(resolve=>setTimeout(resolve,100));}}catch(e){if(mounted.current){setError(asError(e));setPreview(null);}}finally{if(mounted.current){setBusy(false);setRunning(false);setCommitting(false);}}}
 return <Dialog open onOpenChange={open=>{if(!open&&!committing)void cancel();}}><DialogContent className="project-dialog" closeLabel={t.close} showCloseButton={!busy}><DialogHeader><DialogTitle>{title}</DialogTitle><DialogDescription>{p.external}</DialogDescription></DialogHeader><p className="project-path project-confirm-path"><PathView path={project}/></p>
  {error&&<ErrorNotice error={error} t={t}/>}
  {!preview&&!running&&<div className="project-form">
   {operation.action!=="unuse"&&<label>{t.agent}<Choice testId="project-agent" label={t.agent} value={agent} onChange={setAgent} options={[{value:"claude",label:t.claude},{value:"codex",label:t.codex},{value:"cursor",label:t.cursor},{value:"all",label:p.all}]}/></label>}
   {(agent==="cursor"||agent==="all")&&<p className="project-help" data-testid="cursor-editor-support">{p.cursorSupport}</p>}{(agent==="cursor"||agent==="codex"||agent==="all")&&<p className="project-help" data-testid="shared-agent-path"><bdi dir="ltr">.agents/skills</bdi> · {p.sharedPath}</p>}{operation.action==="use"&&<><label>{p.mode}<Choice testId="project-mode" label={p.mode} value={mode} onChange={setMode} options={[{value:"link",label:p.link},{value:"copy",label:p.copy}]}/></label>{operation.bundle?<><label>{p.bundle}<Choice testId="project-bundle" label={p.bundle} value={bundle} onChange={value=>{setBundle(value);setSecondary([]);}} options={bundles.map(b=>({value:b.name,label:b.title}))}/></label><p className="project-help">{p.secondaryHint}</p>{bundles.find(b=>b.name===bundle)?.skills.filter(s=>s.role==="secondary").map(s=><label className="project-check" key={s.id}><Checkbox data-testid={`secondary-${s.id}`} checked={secondary.includes(s.id)} onCheckedChange={checked=>setSecondary(old=>checked?[...old,s.id]:old.filter(id=>id!==s.id))}/><span><bdi>{s.id}</bdi> · {p.manual}</span></label>)}</>:<fieldset><legend>{p.selection}</legend><div className="project-skill-options">{skills.map(s=><label className="project-check" key={s.id}><Checkbox data-testid={`select-${s.id}`} checked={ids.includes(s.id)} onCheckedChange={checked=>setIds(old=>checked?[...old,s.id]:old.filter(id=>id!==s.id))}/><span dir="auto">{visibleName(s.name)}</span><Badge variant="outline">{s.audit_level in t?t[s.audit_level as keyof Text]:s.audit_level}</Badge></label>)}</div></fieldset>}</>}
  </div>}
  {preview&&<div className="project-plan" data-testid="project-preview"><h3>{p.destinations}</h3>{!preview.changed&&<p>{p.noChanges}</p>}{preview.changes.map((change,i)=><article key={i} className="plan-change"><div><bdi>{change.id}</bdi>{change.agent&&<Badge variant="outline">{change.agent}</Badge>}{change.mode&&<Badge variant="secondary">{change.mode==="link"?p.link:p.copy} · {change.invocation==="manual"?p.manual:p.auto}</Badge>}{!change.changed&&<span>{p.unchanged}</span>}</div><PathView path={change.target} project={project}/>{change.source&&<p>{p.from}: <PathView path={change.source}/></p>}</article>)}{preview.candidates.map(c=><article className="plan-candidate" key={c.target}><strong>{c.eligible?p.eligible:p.preserved}</strong><p><PathView path={c.target} project={project}/></p><p dir="auto">{reasonText(c.reason_code,t.locale)}</p>{c.content_hash&&<p dir="ltr">{c.content_hash}</p>}{c.link_to&&<p>→ <PathView path={c.link_to}/></p>}</article>)}{preview.gitignore_suggestions.length>0&&<div className="gitignore-note" data-testid="gitignore-suggestions"><h3>{p.gitignore}</h3><p>{p.gitignoreNote}</p>{preview.gitignore_suggestions.map(pattern=><PathView key={pattern} path={pattern}/>)}</div>}<CodeNotices codes={preview.diagnostic_codes} locale={t.locale}/><p className="reload-note" data-testid="reload-skills">{p.reload}</p></div>}
  {busy&&<div role="status"><p>{committing?p.committing:p.planning}</p><Progress aria-label={t.progress}/></div>}
  <DialogFooter><Button variant="outline" data-testid="cancel-project" disabled={committing||(busy&&!running&&!!preview)} onClick={()=>void cancel()}>{t.cancel}</Button>{preview?<Button data-testid="confirm-project" disabled={busy||!preview.changed} onClick={()=>void apply()}>{p.confirm}</Button>:<Button data-testid="preview-project" disabled={busy||(operation.action==="use"&&(operation.bundle?!bundle:!ids.length))} onClick={()=>void inspect()}>{p.preview}</Button>}</DialogFooter>
 </DialogContent></Dialog>;
}
