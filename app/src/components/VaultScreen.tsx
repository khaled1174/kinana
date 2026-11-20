import {visibleName} from "../lib/paths";
import {CodeNotices} from "./CodeNotices";
import {reasonText} from "../reasons";
import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Search, Plus, Star, FolderOpen, RotateCcw, Package, Layers, X, Check, SlidersHorizontal, Sparkles, RefreshCw } from "lucide-react";
import { api, sourceApi, type OutdatedReport, asError, emptyFilter, type Catalog, type CoreError, type Detail, type Bundle, type Groups, type ClassificationReport } from "../bridge";
import type { Text } from "../i18n";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Popover } from "radix-ui";
import { AuditIndicator } from "./AuditIndicator";
import { ClassifyDialog } from "./ClassifyDialog";
import { skillCount } from "../lib/presentation";
import { Badge } from "./ui/badge";
import { Skeleton } from "./ui/skeleton";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "./ui/tabs";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { ErrorNotice, EmptyNotice, Choice, labelFor } from "./common";
import { SkillDetails } from "./SkillDetails";
import { ImportDialog } from "./ImportDialog";
import { BundleEditor } from "./BundleEditor";
import {SourceUpdateDialog} from "./SourceUpdateDialog";
import {sourceMessages} from "../source-i18n";
export function VaultScreen({t,initialUndo=false}:{t:Text;initialUndo?:boolean}) {
 const sourceText=sourceMessages[t.locale as "ar"|"en"];
 const [sourceReport,setSourceReport]=useState<OutdatedReport|null>(null),[checkedAt,setCheckedAt]=useState<Date|null>(null),[checkingSources,setCheckingSources]=useState(false),[updatingSource,setUpdatingSource]=useState<string|null>(null);
 const sourceId=useRef(""),sourceCancelled=useRef(false),sourceMounted=useRef(true);
 useEffect(()=>{sourceMounted.current=true;return()=>{sourceMounted.current=false;};},[]);
 async function checkSources(){setCheckingSources(true);setError(undefined);sourceCancelled.current=false;const job=crypto.randomUUID();sourceId.current=job;try{await sourceApi.start(job,{action:"check"});while(sourceMounted.current && !sourceCancelled.current){const status=await sourceApi.status(job);if(status.done){if(!sourceMounted.current || sourceCancelled.current)return;if(status.error)throw status.error;setSourceReport(status.report as OutdatedReport);setCheckedAt(new Date());return;}await new Promise(resolve=>setTimeout(resolve,100));}}catch(e){if(sourceMounted.current&&!sourceCancelled.current)setError(asError(e));}finally{try{await sourceApi.cancel(job);}catch(e){if(sourceMounted.current)setError(asError(e));}if(sourceMounted.current)setCheckingSources(false);}}
 async function cancelSourceCheck(){sourceCancelled.current=true;try{await sourceApi.cancel(sourceId.current);}catch(e){setError(asError(e));}}
 const [filter,setFilter]=useState(emptyFilter);const [query,setQuery]=useState("");const [composing,setComposing]=useState(false);
 const [catalog,setCatalog]=useState<Catalog>();const [error,setError]=useState<CoreError>();
 const [loading,setLoading]=useState(true);const [busy,setBusy]=useState(false);const [revision,setRevision]=useState(0);const [selected,setSelected]=useState<string|null>(null);const [detail,setDetail]=useState<Detail>();
 const [tab,setTab]=useState("skills");const [bundles,setBundles]=useState<Bundle[]>([]);const [groups,setGroups]=useState<Groups>();
 const [add,setAdd]=useState<string|null>(null);const [editor,setEditor]=useState<Bundle|null|undefined>(undefined);const [drag,setDrag]=useState(false);
 const [classifying,setClassifying]=useState<string|null>(null);
 const [notice,setNotice]=useState<"saved"|"undoDone"|undefined>(initialUndo?"saved":undefined);const [canUndo,setCanUndo]=useState(initialUndo);const [undoConfirm,setUndoConfirm]=useState(false);
 useEffect(()=>{if(composing)return;const timer=setTimeout(()=>setFilter(f=>({...f,query})),180);return()=>clearTimeout(timer);},[query,composing]);
 useEffect(()=>{let active=true;setLoading(true);setError(undefined);void api.catalog(filter).then(data=>{if(active){setCatalog(data);setSelected(id=>data.items.some(i=>i.skill.id===id)?id:null);}}).catch(e=>{if(active)setError(asError(e));}).finally(()=>{if(active)setLoading(false);});return()=>{active=false;};},[filter,revision]);
 useEffect(()=>{if(!selected){setDetail(undefined);return;}let active=true;setDetail(undefined);void api.detail(selected).then(d=>{if(active)setDetail(d);}).catch(e=>{if(active)setError(asError(e));});return()=>{active=false;};},[selected,revision]);
 useEffect(()=>{let active=true;if(tab==="bundles")void api.bundles().then(d=>{if(active)setBundles(d);}).catch(e=>{if(active)setError(asError(e));});if(tab==="groups")void api.groups().then(d=>{if(active)setGroups(d);}).catch(e=>{if(active)setError(asError(e));});return()=>{active=false;};},[tab,revision]);
 useEffect(()=>{let remove:(()=>void)|undefined;let active=true;void getCurrentWebviewWindow().onDragDropEvent(event=>{if(!active)return;if(event.payload.type==="enter")setDrag(true);else if(event.payload.type==="leave")setDrag(false);else if(event.payload.type==="drop"){setDrag(false);if(event.payload.paths.length===1)setAdd(event.payload.paths[0]);else setError({code:"ONE_SOURCE_REQUIRED",message:t.inputSource});}}).then(fn=>{if(active)remove=fn;else fn();}).catch(e=>setError(asError(e)));return()=>{active=false;remove?.();};},[t]);
 const refresh=useCallback(()=>{setCanUndo(true);setNotice("saved");setRevision(r=>r+1);},[]);
 const classificationSaved=useCallback((report:ClassificationReport)=>{if(report.committed)refresh();},[refresh]);
 async function mutate(action:()=>Promise<unknown>){setBusy(true);setError(undefined);try{const result=await action();if(result&&typeof result==="object"&&"committed" in result&&result.committed===false)return;refresh();}catch(e){setError(asError(e));}finally{setBusy(false);}}
 async function chooseVault(){try{const path=await open({directory:true,multiple:false});if(typeof path==="string"){const data=await api.connect(path);sourceCancelled.current=true;setEditor(undefined);setCatalog(data);setSourceReport(null);setCheckedAt(null);setSelected(null);setFilter(emptyFilter);setQuery("");setCanUndo(false);setNotice(undefined);setRevision(r=>r+1);}}catch(e){setError(asError(e));}}
 async function undo(){setUndoConfirm(false);setBusy(true);try{await api.undo();setNotice("undoDone");setRevision(r=>r+1);}catch(e){setError(asError(e));}finally{setBusy(false);}}
 const chips:{key:string;label:string;remove:()=>void}[]=(["category","agent","audit","state"] as const).flatMap(key=>filter[key]?[{key,label:`${t[key]}: ${labelFor(t,filter[key]!)}`,remove:()=>setFilter(f=>({...f,[key]:null}))}]:[]);
 if(filter.pinned)chips.push({key:"pinned",label:t.favorites,remove:()=>setFilter(f=>({...f,pinned:false}))});
 const options=(all:string,values:string[])=>[{value:"all",label:all},...values.map(value=>({value,label:labelFor(t,value)}))];
 return <div className="vault-screen">
  {drag&&<div className="drop-overlay">{t.dropReady}</div>}
  <div className="vault-toolbar">
   <div className="search-box"><Search aria-hidden="true"/><Input aria-label={t.search} data-testid="search" placeholder={t.searchHint} value={query} onCompositionStart={()=>setComposing(true)} onCompositionEnd={e=>{setComposing(false);setQuery(e.currentTarget.value);}} onChange={e=>setQuery(e.target.value)} dir="auto"/>{query&&<Button size="icon-sm" variant="ghost" aria-label={t.clear} onClick={()=>{setQuery("");setFilter(f=>({...f,query:""}));}}><X/></Button>}</div>
   <Button variant={filter.pinned?"secondary":"outline"} className="favorites-button" aria-pressed={filter.pinned} aria-label={t.favorites} data-testid="filter-pinned" onClick={()=>setFilter({...filter,pinned:!filter.pinned})}><Star className={filter.pinned?"starred":""}/><span>{t.favorites}</span></Button>
   <Popover.Root><Popover.Trigger asChild><Button variant="outline" data-testid="toggle-filters"><SlidersHorizontal/>{t.filter}</Button></Popover.Trigger><Popover.Portal><Popover.Content className="filter-panel" sideOffset={8} align="end" aria-label={t.filter}>
    {(["category","agent","audit","state"] as const).map(key=><div key={key}><span>{t[key]}</span><Choice testId={`filter-${key}`} label={t[key]} value={filter[key]||"all"} options={key==="category"?options(t.allCategories,catalog?.categories||[]):key==="agent"?options(t.allAgents,["claude","codex","cursor"]):key==="audit"?options(t.allAudits,["clean","review","risky","unscanned"]):options(t.allStates,["pending","proposal","approved","original"])} onChange={value=>setFilter(f=>({...f,[key]:value==="all"?null:value}))}/></div>)}
   </Popover.Content></Popover.Portal></Popover.Root>
   <Button variant="outline" aria-label={t.chooseVault} title={catalog?.vault||t.chooseVault} data-testid="choose-vault" onClick={()=>void chooseVault()} disabled={busy}><FolderOpen/></Button>
   <Button variant="outline" data-testid="check-source-updates" className="check-source-button" title={sourceText.check} aria-label={sourceText.check} disabled={busy||checkingSources||!catalog} onClick={()=>void checkSources()}><RefreshCw/><span>{sourceText.check}</span></Button>
   <Button data-testid="add-skill" onClick={()=>setAdd("")} disabled={busy||!catalog} variant="outline"><Plus data-icon="inline-start"/>{t.add}</Button>
  </div>
  {checkingSources&&<div className="source-check-status" role="status"><span>{sourceText.checking}</span><Button size="sm" variant="ghost" data-testid="cancel-source-check" onClick={()=>void cancelSourceCheck()}>{t.cancel}</Button></div>}
  {sourceReport&&checkedAt&&<details className="source-check-results" data-testid="source-check-results" data-checked-at={checkedAt.getTime()}><summary>{sourceText.lastCheck}: {checkedAt.toLocaleTimeString(t.locale)}{!sourceReport.complete&&` · ${sourceText.partial}`}</summary><ul>{sourceReport.items.map(item=><li key={item.id}><bdi>{item.id}</bdi> · {reasonText(item.reason_code,t.locale)}</li>)}</ul></details>}
  {error&&<div className="screen-notice"><ErrorNotice t={t} error={error}/></div>}
  {notice&&<div className="save-notice" role="status"><span><Check/>{t[notice]}</span>{canUndo&&<Button size="sm" variant="ghost" data-testid="undo" disabled={busy} onClick={()=>setUndoConfirm(true)}><RotateCcw data-icon="inline-start"/>{t.undo}</Button>}</div>}
  {!catalog ? loading?<div className="loading-list" aria-label={t.loading}><Skeleton className="h-16"/><Skeleton className="h-16"/><Skeleton className="h-16"/></div>:<EmptyNotice title={t.noVault} body={t.noVaultBody}><Button variant="outline" onClick={()=>void chooseVault()}>{t.chooseVault}</Button></EmptyNotice> : <>
  <Tabs value={tab} onValueChange={setTab} className="vault-tabs"><div className="vault-tabs-header"><TabsList variant="line"><TabsTrigger value="skills" data-testid="vault-tab-skills">{t.skills}<span className="tab-count" title={skillCount(catalog.total,t.locale)}>{new Intl.NumberFormat(t.locale).format(catalog.total)}</span></TabsTrigger><TabsTrigger value="bundles" data-testid="vault-tab-bundles"><Package/>{t.bundles}</TabsTrigger><TabsTrigger value="groups" data-testid="vault-tab-groups"><Layers/>{t.groups}</TabsTrigger></TabsList></div>
  <TabsContent value="skills" className="skills-tab">
   <div className="results-toolbar"><div className="filter-chips">{chips.map(chip=><Button key={chip.key} variant="secondary" size="sm" className="filter-chip" data-filter={chip.key} aria-label={`${t.removeFilter}: ${chip.label}`} onClick={chip.remove}>{chip.label}<X/></Button>)}</div><span data-testid="result-count" data-count={catalog.items.length}>{catalog.items.length===catalog.total?skillCount(catalog.total,t.locale):`${skillCount(catalog.items.length,t.locale)} ${t.total} ${skillCount(catalog.total,t.locale)}`}</span>{(chips.length>0||query)&&<Button size="sm" variant="ghost" data-testid="clear-filters" onClick={()=>{setQuery("");setFilter(emptyFilter);}}>{t.clear}</Button>}</div>
   <CodeNotices codes={catalog.diagnostic_codes} locale={t.locale}/>
   <div className="catalog-layout"><section className="skill-list" aria-label={t.skills} aria-busy={loading}>{loading&&<div className="list-loading" role="status">{t.loading}</div>}{catalog.items.length?catalog.items.map(({skill,state,classified_by})=><article className="skill-row" key={skill.id} data-selected={selected===skill.id} data-testid={`skill-${skill.id}`}><button type="button" className="skill-select" disabled={loading} onClick={()=>setSelected(skill.id)} aria-pressed={selected===skill.id}><div className="row-top"><strong dir="auto" title={visibleName(skill.name)}>{visibleName(skill.name)}</strong><AuditIndicator level={skill.audit_level} t={t}/></div><p className="row-description" dir="auto">{skill.description}</p></button><div className="row-meta">{sourceReport?.items.find(item=>item.id===skill.id)?.status==="outdated"&&<Badge variant="secondary" data-testid={`source-update-${skill.id}`}>{sourceText.available}</Badge>}<Badge variant="outline" title={skill.category}>{labelFor(t,skill.category)}</Badge>{classified_by==="keywords"?<><Badge variant="secondary" className="approximate-badge" data-testid={`approximate-${skill.id}`}>{t.approximate}</Badge><Button size="sm" variant="ghost" className="classify-row-button" data-testid={`classify-${skill.id}`} disabled={busy||loading} onClick={()=>setClassifying(skill.id)}><Sparkles/>{t.classifyAgent}</Button></>:<span className={state==="proposal"?"pending-label":""}>{labelFor(t,state)}</span>}</div><Button variant="ghost" size="icon-sm" className="row-pin" aria-label={`${skill.pinned?t.unpin:t.pin}: ${visibleName(skill.name)}`} data-testid={`pin-${skill.id}`} disabled={busy||loading} onClick={()=>void mutate(()=>api.pin(skill.id,!skill.pinned))}><Star className={skill.pinned?"starred":""}/></Button></article>):<EmptyNotice title={catalog.total?t.noResults:t.emptyVault} body={catalog.total?t.noResultsBody:t.emptyVaultBody}/>}</section>
    {selected ? detail?<SkillDetails t={t} data={detail} categories={catalog.categories} busy={busy} onChange={mutate} onClose={()=>setSelected(null)} onClassify={setClassifying} onUpdate={setUpdatingSource} sourceUpdateAvailable={sourceReport?.items.find(item=>item.id===selected)?.status==="outdated"}/>:<div className="detail-loading" role="status"><Skeleton className="h-10"/><Skeleton className="h-40"/></div>:<section className="empty-detail"><EmptyNotice title={t.selectSkill} body={t.selectBody}/></section>}
   </div>
  </TabsContent>
  <TabsContent value="bundles" className="bundles-tab"><div className="section-toolbar"><Button data-testid="new-bundle" variant="outline" onClick={()=>setEditor(null)}><Plus data-icon="inline-start"/>{t.newBundle}</Button></div>{bundles.length?<div className="bundle-list">{bundles.map(bundle=><article className="bundle-card" key={bundle.name}><div><h2 dir="auto">{bundle.title}</h2><code>{bundle.name}</code></div><p>{skillCount(bundle.skills.length,t.locale)} · <bdi>{bundle.agents.join(" / ")}</bdi></p><ul>{bundle.skills.map(s=><li key={s.id}><bdi>{s.id}</bdi><span>{labelFor(t,s.role)}{s.group&&` · ${s.group}`}</span></li>)}</ul><Button size="sm" variant="outline" onClick={()=>setEditor(bundle)}>{t.editBundle}</Button></article>)}</div>:<EmptyNotice title={t.noBundles}/>}</TabsContent>
  <TabsContent value="groups" className="groups-tab"><p className="notice">{t.groupHint}</p><CodeNotices codes={groups?.diagnostic_codes} locale={t.locale}/>{groups?.candidates.length?<div className="bundle-list">{groups.candidates.map((group,i)=><article key={i} className="bundle-card"><h2><bdi>{group.skill_ids.join(" + ")}</bdi></h2><Badge variant="outline">{group.category?labelFor(t,group.category):t.none}</Badge><p>{reasonText(group.reason_code||"OVERLAP_CANDIDATE",t.locale)}</p><Button variant="outline" size="sm" onClick={()=>{setEditor({name:"",title:"",agents:["claude","codex"],skills:group.skill_ids.map((id,i)=>({id,role:i===0?"primary":"secondary",group:"function"}))});}}>{t.newBundle}</Button></article>)}</div>:<EmptyNotice title={t.noGroups}/>}</TabsContent>
  </Tabs>
  </>}
  {updatingSource!==null&&<SourceUpdateDialog id={updatingSource} t={t} onClose={()=>setUpdatingSource(null)} onSaved={()=>{setSourceReport(null);setCheckedAt(null);refresh();}}/>}
  {classifying!==null&&<ClassifyDialog id={classifying} t={t} onClose={()=>setClassifying(null)} onSaved={classificationSaved}/>}
  {add!==null&&<ImportDialog t={t} initial={add} onClose={()=>setAdd(null)} onSaved={refresh}/>}
  {editor!==undefined&&catalog&&<BundleEditor key={catalog.vault} t={t} initial={editor} vault={catalog.vault} onClose={()=>setEditor(undefined)} onSaved={refresh}/>}
  <Dialog open={undoConfirm} onOpenChange={setUndoConfirm}><DialogContent closeLabel={t.close}><DialogHeader><DialogTitle>{t.confirmUndo}</DialogTitle><DialogDescription>{t.confirmUndoBody}</DialogDescription></DialogHeader><DialogFooter><Button variant="outline" onClick={()=>setUndoConfirm(false)}>{t.cancel}</Button><Button data-testid="confirm-undo" onClick={()=>void undo()}>{t.undo}</Button></DialogFooter></DialogContent></Dialog>
 </div>;
}
