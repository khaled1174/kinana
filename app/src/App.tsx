import { useEffect, useState, lazy, Suspense } from "react";
import { Package2, FolderKanban, Settings2, Languages } from "lucide-react";
import { Direction } from "radix-ui";
import { cn } from "cn";
import {ErrorNotice} from "./components/common";
import { api, type CoreError } from "./bridge";
import { messages, type Language } from "./i18n";
import { Button } from "./components/ui/button";
import { skillCount } from "./lib/presentation";
const SettingsScreen=lazy(()=>import("./components/SettingsScreen").then(m=>({default:m.SettingsScreen})));
import { PathContext } from "./components/PathView";
import { ProjectsScreen } from "./components/ProjectsScreen";
import { VaultScreen } from "./components/VaultScreen";
type Page="vault"|"projects"|"settings";
export function App() {
 const [startupError,setStartupError]=useState<CoreError|null>(null);
 const [home,setHome]=useState("");const [firstRun,setFirstRun]=useState(false);const [routerMoved,setRouterMoved]=useState<{from:string;to:string}|null>(null);const [openRouter,setOpenRouter]=useState(0);const [auditRules,setAuditRules]=useState<{older:number}|null>(null);const [openAudit,setOpenAudit]=useState(0);const [revision,setRevision]=useState(0);const [undoAvailable,setUndoAvailable]=useState(false);
 const [language,setLanguage]=useState<Language>("ar");const [page,setPage]=useState<Page>("vault");const [connected,setConnected]=useState(false);const t=messages[language];const direction=language==="ar"?"rtl":"ltr";
 useEffect(()=>{document.documentElement.lang=language;document.documentElement.dir=direction;},[language,direction]);
 useEffect(()=>{void api.bootstrap().then(data=>{setConnected(true);setHome(data.home);setFirstRun(data.first_run);setStartupError(data.startup_error);setRouterMoved(data.router_moved??null);setAuditRules(data.audit_rules??null);if(data.first_run)setPage("settings");}).catch(()=>setConnected(false));},[]);
 const navigation=[{page:"vault",icon:Package2},{page:"projects",icon:FolderKanban},{page:"settings",icon:Settings2}] as const;
 return <Direction.Provider dir={direction}><PathContext.Provider value={{home}}><div className="app-shell">
  <aside className="sidebar"><div className="brand"><div className="brand-mark"><Package2/></div><div><strong dir="ltr">{t.app}</strong><p>{t.caption}</p></div></div><p className="nav-label">{t.workspace}</p><nav aria-label={t.navigation}>{navigation.map(({page:target,icon:Icon})=><Button variant="ghost" key={target} className={cn("nav-button",page===target&&"nav-active")} aria-current={page===target?"page":undefined} data-testid={`nav-${target}`} onClick={()=>setPage(target)}><Icon data-icon="inline-start"/><span>{t[target]}</span></Button>)}</nav><div className="sidebar-bottom"><span className="build-label">{t.phase}</span><Button variant="ghost" className="language-button" data-testid="language" onClick={()=>setLanguage(language==="ar"?"en":"ar")}>{t.language}<Languages/></Button></div></aside>
  <main><header className="page-header"><h1 data-testid="page-title">{t[page]}</h1><span className={cn("connection-dot",connected&&"ready")} role="status" aria-label={connected?t.connected:t.error} title={connected?t.connected:t.error} data-testid="connection"/></header>{startupError&&<ErrorNotice error={startupError} t={t}/>} {routerMoved&&!firstRun&&<div className="notice router-moved" role="status" data-testid="router-moved"><p>{t.routerMoved}</p><Button variant="outline" data-testid="router-moved-review" onClick={()=>{setRouterMoved(null);setPage("settings");setOpenRouter(n=>n+1);}}>{t.routerMovedReview}</Button></div>} {auditRules&&!firstRun&&<div className="notice router-moved" role="status" data-testid="audit-rules-older"><p>{skillCount(auditRules.older,t.locale)} {t.auditRulesOlder}</p><div className="button-row"><Button variant="outline" data-testid="audit-rules-preview" onClick={()=>{setAuditRules(null);void api.answerAuditRules().catch(()=>undefined);setPage("settings");setOpenAudit(n=>n+1);}}>{t.auditRulesPreview}</Button><Button variant="ghost" data-testid="audit-rules-later" onClick={()=>{setAuditRules(null);void api.answerAuditRules().catch(()=>undefined);}}>{t.auditRulesLater}</Button></div></div>} {page==="vault"?<VaultScreen key={revision} t={t} initialUndo={undoAvailable}/>:page==="projects"?<ProjectsScreen t={t} home={home} onImported={committed=>{setRevision(n=>n+1);setUndoAvailable(committed);}}/>:<Suspense fallback={<p role="status" className="notice">{t.loading}</p>}><SettingsScreen t={t} firstRun={firstRun} openRouter={openRouter} openAudit={openAudit} onChanged={committed=>{setRevision(n=>n+1);setUndoAvailable(committed);}} onFinished={()=>{setFirstRun(false);setRevision(n=>n+1);setPage("vault");}}/></Suspense>}</main>
 </div></PathContext.Provider></Direction.Provider>;
}
