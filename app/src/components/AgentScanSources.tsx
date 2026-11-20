import {useState} from "react";
import {FolderSearch,Plus} from "lucide-react";
import {settingsApi,asError,type AgentSkillLocations,type CoreError} from "../bridge";
import type {Text} from "../i18n";
import {settingsMessages} from "../settings-i18n";
import {reasonText} from "../reasons";
import {Button} from "./ui/button";
import {Checkbox} from "./ui/checkbox";
import {Textarea} from "./ui/textarea";
import {ErrorNotice} from "./common";
import {PathView} from "./PathView";

export function AgentScanSources({t,disabled,roots,onRootsChange,onAddRoot,onScan}:{
 t:Text;disabled:boolean;roots:string;onRootsChange:(value:string)=>void;
 onAddRoot:()=>void;onScan:(roots:string[])=>void;
}) {
 const p=settingsMessages[t.locale as "ar"|"en"];
 const [selected,setSelected]=useState(["claude","codex","cursor"]);
 const [locations,setLocations]=useState<AgentSkillLocations>();
 const [finding,setFinding]=useState(false),[error,setError]=useState<CoreError>();
 const manual=roots.split(/\r?\n/).map(s=>s.trim()).filter(Boolean);
 const name=(agent:string)=>agent==="claude"?t.claude:agent==="codex"?t.codex:t.cursor;
 async function find(){
  setFinding(true);setError(undefined);
  try{const result=await settingsApi.agentLocations(selected);setLocations(result);if(result.roots.length)onScan(result.roots);}
  catch(e){setError(asError(e));}finally{setFinding(false);}
 }
 return <div className="settings-fields agent-scan-sources">
  <p className="settings-help">{p.autoScanHint}</p>
  <fieldset disabled={disabled||finding} className="scan-agent-choices">
   <legend>{p.chooseScanAgents}</legend>
   {["claude","codex","cursor"].map(agent=><label key={agent} className="checkbox-label"><Checkbox data-testid={`scan-agent-${agent}`} checked={selected.includes(agent)} onCheckedChange={checked=>{setSelected(old=>checked?[...old,agent]:old.filter(a=>a!==agent));setLocations(undefined);setError(undefined);}}/>{name(agent)}</label>)}
  </fieldset>
  {error&&<ErrorNotice error={error} t={t}/>}
  <Button data-testid="scan-agent-skills" disabled={disabled||finding||!selected.length} onClick={()=>void find()}><FolderSearch/>{finding?p.discoveringAgents:p.scanMySkills}</Button>
  {locations&&<div className="scan-agent-locations" data-testid="scan-agent-locations" aria-live="polite">
   {!locations.roots.length&&<p role="status">{p.noAgentLocations}</p>}
   {locations.locations.map(location=><div key={location.path}><PathView path={location.path}/><span className="settings-help">{location.agents.map(name).join(" / ")} · {location.status==="ready"?p.locationReady:location.status==="missing"?p.locationMissing:reasonText(location.reason_code,t.locale)}</span></div>)}
  </div>}
  <details className="scan-manual-sources">
   <summary data-testid="manual-scan-toggle">{p.manualScan}</summary>
   <div className="settings-fields"><label>{p.scanPaths}<Textarea rows={3} data-testid="settings-scan-paths" disabled={disabled} value={roots} onChange={e=>onRootsChange(e.target.value)} dir="ltr"/></label>
   <p className="settings-help">{p.scanHint}</p><div className="button-row"><Button variant="outline" disabled={disabled} onClick={onAddRoot}><Plus/>{p.addRoot}</Button><Button data-testid="settings-scan" disabled={disabled||!manual.length} onClick={()=>onScan(manual)}>{p.scan}</Button></div></div>
  </details>
 </div>;
}
