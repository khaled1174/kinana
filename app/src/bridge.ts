import type {RulesUpgrade} from "./lib/audit-rules";
import { invoke } from "@tauri-apps/api/core";
export type CoreError = { code: string; reason_code?:string; message: string };
export type Agent = {name:string;available:boolean;project_skills:string;global_skills:string};
export type Bootstrap = {version:string;agents:Agent[];home:string;vault:string;first_run:boolean;startup_error:CoreError|null;router_moved:{from:string;to:string}|null;audit_rules:{older:number;rules_version:number}|null};
export type Source = {kind:string;url:string|null;subpath:string|null;commit:string|null;tracking_ref?:string;tracking_kind?:string;found_in:string[]};
export type Skill = {id:string;pinned:boolean;name:string;description:string;category:string;stack:string[];stage:string[];when_to_use:string;audit_level:string;content_hash:string;added_at:string;role:string;source:Source;tags:string[]};
export type Proposal = {category_reason?:string;category:string;stack?:string[];stage?:string[];role:string;when_to_use:string;when_not:string;improved_description:string;tags:string[];overlaps_with:string[];confidence:number};
export type Metadata = {classification_context?:{prompt_version:number;category_reason:string};id:string;pinned:boolean;classification:{category:string;stack?:string[];stage?:string[];tags:string[];role:string;when_to_use:string;when_not:string;classified_by:string};description:{original:string;improved:string|null;active:string;approved_at:string|null};audit:Audit;proposal_source_changed?:boolean;proposal?:Proposal};
export type Filter = {query:string;category:string|null;stack?:string|null;stage?:string|null;audit:string|null;agent:string|null;state:string|null;pinned:boolean};
export const emptyFilter:Filter={query:"",category:null,audit:null,agent:null,state:null,pinned:false};
export type Catalog = {vault:string;categories:string[];stacks:string[];stages:string[];total:number;items:{skill:Skill;state:string;classified_by:string}[];warnings:string[];diagnostic_codes?:string[]};
export type Detail = {skipped_classification?:{agent:string;reason_code:string;skipped_at:string};classification_history_unavailable?:boolean;frontmatter:string;markdown_body:string;detail:{skill:Skill;metadata:Metadata;raw_skill_md:string;locations:string[];variants:{path:string;content_hash:string|null;status:string}[]};projects:string[];warnings:string[];diagnostic_codes?:string[]};
export type Bundle = {name:string;title:string;agents:string[];skills:{id:string;role:string;group?:string;audit_level?:string}[];risky_members?:number};
export type Groups = {candidates:{skill_ids:string[];category:string|null;reasons:string[];reason_code?:string}[];warnings:string[];diagnostic_codes?:string[]};
export type ImportReport = {source:string;dry_run:boolean;requires_preparation:boolean;items:{id:string|null;name:string|null;path:string;status:string;content_hash:string|null;review_token:string|null;description?:string|null;audit:Audit;selected:boolean;error:string|null;reason_code?:string|null}[];imported:string[];duplicates:string[];variants:string[];committed:boolean;warnings:string[];diagnostic_codes?:string[]};
export type ImportStatus = {prepared:boolean;stage:string;done:boolean;report:ImportReport|null;error:CoreError|null};
export type ClaudeSettings = {model:string;effort:string};
export type TokenEstimate = {claude?:ClaudeSettings;fixed_tokens_per_call:number;fixed_tokens_total:number;skill_tokens_total:number;total_input_tokens:number;retry_limit_input_tokens:number;calibration:{source_log:string;claude_version:string}|null};
export type ProviderStatus = {agent:string;status:string;reason_code?:string;planned_calls:number};
export type ClassificationStop = {reason_code:string;message:string;classified:number;remaining:string[];providers?:ProviderStatus[]};
export type PiConfirmation = {provider:string;model:string;model_default:boolean;skills:number;max_calls:number;token:string};
export type AutoApproval = {ids:string[];commit:string};
export type PiModel = {id:string;context:string;max_out:string;thinking:boolean;images:boolean};
export type PiProvider = {id:string;models:PiModel[];default_model:string|null;default_listed:boolean};
export type PiModelList = {version:string;providers:PiProvider[];skipped_rows:number};
export type SkippedClassification = {id:string;agent:string;reason_code:string;attempts:number};
export type ClassificationReport = {agent?:string;providers?:ProviderStatus[];stopped?:ClassificationStop;skipped?:SkippedClassification[];pi_confirmation?:PiConfirmation;auto_approved?:AutoApproval;committed:boolean;dry_run:boolean;selected:number;estimated_agent_calls:number;estimated_tokens:TokenEstimate;agent_calls:number;items:{id:string;classified_by:string;proposal_created:boolean;warnings:string[];reason_code?:string;searched_locations?:string[];diagnostic_codes?:string[]}[];warnings:string[];diagnostic_codes?:string[]};
export type PendingDescription = {id:string;name:string;current:string;proposed:string;source_changed:boolean};
export type BatchReview = {decision:string;items:{id:string;original:string;current:string;proposed:string;active:string}[];preview_token:string;requires_confirmation:boolean;dry_run:boolean;committed:boolean};
export type ClassificationProgress = {total:number;completed:number;skipped:number;current_id:string|null};
function reviewedItems(report:ImportReport, select:string[]) {
  const paths=new Set(select);
  return Object.fromEntries(report.items.filter(item=>paths.has(item.path)).map(item=>[item.path,[item.content_hash,item.review_token]]));
}
export const api = {
  bootstrap:()=>invoke<Bootstrap>("bootstrap"),
  catalog:(filter:Filter)=>invoke<Catalog>("catalog",{filter}),
  connect:(path:string)=>invoke<Catalog>("connect_vault",{path}),
  detail:(id:string)=>invoke<Detail>("skill_detail",{id}),
  pin:(id:string,pinned:boolean)=>invoke<{changed:boolean;committed:boolean}>("pin_skill",{id,pinned}),
  category:(id:string,category:string)=>invoke<{committed:boolean}>("category_skill",{id,category}),
  review:(id:string,decision:string)=>invoke<{committed:boolean}>("review_skill",{id,decision}),
  pendingDescriptions:()=>invoke<PendingDescription[]>("pending_descriptions"),
  previewAcceptMany:(ids:string[])=>invoke<BatchReview>("preview_accept_descriptions",{ids}),
  acceptMany:(ids:string[],token:string)=>invoke<BatchReview>("accept_descriptions",{ids,token}),
  undo:()=>invoke("undo_vault"),
  undoAutoApproval:(commit:string)=>invoke("undo_auto_approval",{commit}),
  piModels:()=>invoke<PiModelList>("pi_models"),
  answerAuditRules:()=>invoke<void>("answer_audit_rules_prompt"),
  previewClassify:(id:string|undefined,all=false,agent?:string,ids?:string[],pending=false,batchChoice=false)=>invoke<ClassificationReport>("preview_classify",{id:id??null,all,pending,ids:ids??null,agent:agent??null,batchChoice}),
  startClassify:(id:string|undefined,estimate:TokenEstimate,all=false,agent?:string,ids?:string[],piConfirmation?:string,pending=false,batchChoice=false)=>invoke<void>("start_classify",{id:id??null,all,pending,estimate,confirmed:true,ids:ids??null,agent:agent??null,piConfirmation:piConfirmation??null,batchChoice}),
  classifyStatus:()=>invoke<{done:boolean;committing:boolean;progress:ClassificationProgress;report:ClassificationReport|null;error:CoreError|null}>("classify_status"),
  cancelClassify:()=>invoke<boolean>("cancel_classify"),
  reveal:(id:string)=>invoke<void>("reveal_skill_folder",{id}),
  bundles:()=>invoke<Bundle[]>("vault_bundles"),
  groups:()=>invoke<Groups>("vault_groups"),
  saveBundle:(bundle:Bundle,editing:boolean,dryRun:boolean)=>invoke<{committed:boolean;warnings:string[];diagnostic_codes?:string[]}>("save_bundle",{bundle:{name:bundle.name,title:bundle.title,agents:bundle.agents,skills:bundle.skills.map(({id,role,group})=>({id,role,group}))},editing,dryRun}),
  previewImport:(input:string)=>invoke<ImportReport>("preview_import",{input}),
  prepareImport:(input:string)=>invoke<void>("prepare_import",{input}),
  importStatus:()=>invoke<ImportStatus>("import_status"),
  cancelImport:()=>invoke<boolean>("cancel_import"),
  commitImport:(report:ImportReport,select:string[],allowRisky:boolean,suffix:string|null)=>invoke<void>("commit_import",{selection:{reviewed:reviewedItems(report,select),select,allow_risky:allowRisky,suffix}}),
};
export const bootstrap=api.bootstrap;
export function asError(value:unknown):CoreError {return typeof value==="object" && value!==null && "code" in value && "message" in value ? value as CoreError : {code:"DESKTOP_ERROR",message:String(value)};}
export type Project = {path:string;name:string;state:string;bundle:string|null;activations:{id:string;name:string;agent:string;mode:string;invocation:string;target:string;state:string;message:string;reason_code:string}[];local:{agent:string;entry:{path:string;name:string|null;status:string;skill_id:string|null;content_hash:string|null;error:string|null}}[];warnings:string[];diagnostic_codes?:string[]};
export type ProjectRequest = {action:"use";project:string;selections:string[];agent:string;mode:string;include_secondary:string[]}|{action:"unuse";project:string;ids:string[];agent:string}|{action:"adopt";project:string;agent:string};
export type ActivationRisk={id:string;role:string;audit_level:"risky";rules_version:number|null;details:Audit["details"]};
export type ProjectPreview = {risky:ActivationRisk[];request:ProjectRequest;token:string;changed:boolean;changes:{target:string;source:string|null;id:string|null;agent:string|null;mode:string|null;invocation:string|null;changed:boolean}[];candidates:{target:string;agent:string;id:string|null;eligible:boolean;reason:string;reason_code:string;content_hash:string|null;link_to:string|null}[];warnings:string[];diagnostic_codes?:string[];gitignore_suggestions:string[]};
export const projectApi = {
 list:(selected:string[])=>invoke<Project[]>("projects",{selected}),
 preview:(request:ProjectRequest)=>invoke<ProjectPreview>("preview_project",{request}),
 start:(request:ProjectRequest,token:string,acknowledgeRisk=false)=>invoke<void>("start_project",{request,token,confirmed:true,acknowledgeRisk}),
 status:()=>invoke<{done:boolean;committing:boolean;report:ProjectPreview|null;error:CoreError|null}>("project_status"),
 cancel:()=>invoke<boolean>("cancel_project"),
};

export type Audit={level:string;findings:string[];rules_version:number;details:{rule:string;strength:string;level:string;reason_code:string;path:string;line:number|null;end_line:number|null;excerpt:string|null;forms?:string[]}[]};
export type Definitions={categories?:Record<string,string>;stacks?:Record<string,string>;stages?:Record<string,string>;rules?:string[]|null};
export type PiSettings={enabled:boolean;provider:string;model:string;thinking:string;account:string};
export type Config={classification_claude?:ClaudeSettings;classification_pi?:PiSettings;version:number;classification_agent:string;record_found_in:boolean;categories:string[];stacks:string[];stages:string[];scan_paths:string[];detected_agents:string[];definitions?:Definitions;auto_approve_descriptions?:boolean};
export type TaxonomyEntry={value:string;definition:string;default:string|null;customized:boolean};
export type PromptPreview={prompt_version:number;definitions_sent:boolean;text:string;arguments:string[];template_hash:string;skill_text_placeholder:string;skill_data_placeholder:string;delimiter_placeholder:string};
export type TaxonomyView={effective:{categories:TaxonomyEntry[];stack_rule:string;stacks:TaxonomyEntry[];stages:TaxonomyEntry[];rules:string[];default_rules:string[];rules_customized:boolean};preview:PromptPreview|null;error:CoreError|null};
export type Settings={migration_required:boolean;vault_location_codes:string[];vault:string;index:string;home:string;preferences:string;first_run:boolean;path_conflict:boolean;config:Config|null;error:CoreError|null;agents:Agent[];remote:string|null;backups:string[]};
export type SettingsAction={kind:"initialize"|"select_vault";path:string}|{kind:"save";classification_agent:string;categories:string[];stacks?:string[];stages?:string[];scan_paths:string[];record_found_in:boolean;definitions?:Definitions;pi?:PiSettings;claude?:ClaudeSettings;auto_approve_descriptions?:boolean}|{kind:"rebuild_index"|"install_router"|"refresh_audit"}|{kind:"remote";url:string};
// Reasons core gives for each planned settings change. UPGRADE_ROUTER (8.4): an owned, unmodified earlier router text is replaced by the current one.
// RELOCATE_ROUTER: the app moved and the router's CLI path is rewritten; ROUTER_PREVIOUS_CLI_PATH and ROUTER_CLI_PATH name both ends.
export type SettingsChangeReason="INITIALIZE_VAULT"|"INITIALIZE_INDEX"|"SELECT_DEFAULT_VAULT"|"SAVE_VAULT_SETTINGS"|"REBUILD_INDEX"|"INSTALL_ROUTER"|"UPGRADE_ROUTER"|"RELOCATE_ROUTER"|"ROUTER_PREVIOUS_CLI_PATH"|"ROUTER_CLI_PATH"|"REFRESH_AUDIT"|"CONFIGURE_REMOTE";
export type SettingsPlan={action:SettingsAction;token:string;changed:boolean;changes:{path:string;reason_code:SettingsChangeReason|(string&{})}[];diagnostic_codes:string[];audit?:RulesUpgrade};
export type Doctor={healthy:boolean;checks:{name:string;passed:boolean;reason_code:string;message:string}[]};
export type AgentSkillLocations={roots:string[];locations:{path:string;agents:string[];status:"ready"|"missing"|"blocked";reason_code:string}[]};
export const settingsApi={
 guide:(draft:{categories:string[];definitions:Definitions}|null)=>invoke<TaxonomyView>("classification_guide",{draft}),
 agentLocations:(agents:string[])=>invoke<AgentSkillLocations>("agent_skill_locations",{agents}),
 read:()=>invoke<Settings>("desktop_settings"),location:(path:string)=>invoke<string[]>("vault_location_warnings",{path}),preview:(action:SettingsAction)=>invoke<SettingsPlan>("preview_settings",{action}),
 start:(action:SettingsAction,token:string)=>invoke<void>("start_settings",{action,token,confirmed:true}),status:()=>invoke<{done:boolean;committed:boolean;committing:boolean;vault:string|null;error:CoreError|null}>("settings_status"),cancel:()=>invoke<boolean>("cancel_settings"),doctor:()=>invoke<Doctor>("desktop_doctor"),
 scan:(roots:string[],reuse=false)=>invoke<void>("start_scan",{roots,reuse}),scanStatus:()=>invoke<{done:boolean;imported:boolean;stage:string;completed:number;total:number;report:ImportReport|null;error:CoreError|null}>("scan_status"),cancelScan:()=>invoke<boolean>("cancel_scan"),previewScanSelection:(report:ImportReport,select:string[],allowRisky:boolean,primary:string[],descriptions:Record<string,string>)=>invoke<string>("preview_scan_selection",{selection:{reviewed:reviewedItems(report,select),select,allow_risky:allowRisky,suffix:null,primary,descriptions}}),
 commitScan:(report:ImportReport,select:string[],allowRisky:boolean,primary:string[],descriptions:Record<string,string>,approvalToken:string)=>invoke<void>("commit_scan",{selection:{reviewed:reviewedItems(report,select),select,allow_risky:allowRisky,suffix:null,primary,descriptions,approval_token:approvalToken},confirmed:true}),
};
export type SourceStatus={id:string;status:string;reason_code:string;repository:string|null;reference:string|null;tracking_kind:string|null;old_commit:string|null;new_commit:string|null;content_changed:boolean|null;error:CoreError|null};
export type OutdatedReport={items:SourceStatus[];groups_fetched:number;complete:boolean};
export type SourceFileInfo={kind:string;mode:number;bytes:number;content_hash:string|null};
export type SourceUpdateReport={id:string;repository:string;reference:string;tracking_kind:string;old_commit:string;new_commit:string;status:string;old_content_hash:string;new_content_hash:string;files:{path:string;change:string;old:SourceFileInfo|null;new:SourceFileInfo|null;unified_diff:string|null;truncated:boolean}[];old_audit:Audit;new_audit:Audit;warnings:{reason_code:string;old_description:string|null;new_description:string|null}[];preview_token:string;dry_run:boolean;requires_confirmation:boolean;changed:boolean;committed:boolean;diff_byte_limit:number;omitted_files:number};
export type SourceUpdateOptions={id:string;reference?:string;confirm?:string;allow_risky?:boolean;dry_run?:boolean};
export const sourceApi={
 start:(id:string,request:{action:"check"}|{action:"update";options:SourceUpdateOptions})=>invoke<void>("start_sources",{id,request}),
 status:(id:string)=>invoke<{done:boolean;committing:boolean;stage:string;report:OutdatedReport|SourceUpdateReport|null;error:CoreError|null}>("sources_status",{id}),
 cancel:(id:string)=>invoke<boolean>("cancel_sources",{id}),
};
