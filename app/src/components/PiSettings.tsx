import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { api, asError, type CoreError, type PiModelList, type PiSettings as Settings } from "../bridge";
import type { Text } from "../i18n";
import { reasonText } from "../reasons";
import { Button } from "./ui/button";
import { Checkbox } from "./ui/checkbox";
import { Input } from "./ui/input";
import { Choice } from "./common";
export const emptyPi:Settings={enabled:false,provider:"",model:"",thinking:"",account:""};
const PI_DEFAULT="__pi_default__",NONE="__none__";
const words={
 ar:{title:"Pi — تجريبي",enable:"استخدم Pi للتصنيف (مطفأ افتراضيًا)",terms:"استخدام اشتراكات المزوّدين عبر Pi يخضع لشروط كل مزوّد، والمسؤولية على المستخدم.",help:"يُشغَّل Pi بعد Claude في الوضع التلقائي بعد موافقتك الصريحة في كل تشغيل، أو حين تختاره وكيلًا. Pi لا يعرض قائمة أدواته عند بدء الجلسة، لذلك يعمل كنانة ببوابة بديلة اعتمدتها: إصدار مراجَع فقط (1.5.2)، وخيارات تعطّل الأدوات والإضافات والمهارات وملفات السياق والجلسة والشبكة عند البدء، وإيقاف فوري عند أي حدث أداة أو حدث غير معروف. كنانة لا يقرأ توكنات Pi ولا ملف auth.json.",provider:"المزوّد (مطلوب مع تفعيل Pi)",model:"الموديل (اختياري)",modelManual:"الموديل (اختياري: اتركه فارغًا لموديل Pi الافتراضي لهذا المزوّد)",thinking:"مستوى التفكير",account:"نوع الحساب",default:"افتراضي Pi",unspecified:"غير محدد",subscription:"اشتراك",apiKey:"مفتاح API",cache:"تنبيه: إذا كان تسخين الكاش (cacheWarming) مفعّلًا في إعدادات Pi فقد يرسل طلبات إضافية للنموذج لا يتحكم فيها كنانة.",
  reading:"جارٍ قراءة المزوّدين والموديلات من Pi…",read:(version:string,count:number)=>`من Pi ${version}: ${count} ${count===1?"مزوّد":"مزوّدين"} جاهزة. القائمة من Pi نفسه بأمر قراءة لا يرسل نصًا ولا يستدعي نموذجًا.`,empty:"لم يذكر Pi أي مزوّد جاهز. سجّل الدخول في Pi أو اضبط مفتاحًا ثم أعد القراءة، أو اكتب المزوّد والموديل يدويًا.",failed:"تعذّرت قراءة القائمة من Pi: ",manual:"اكتب المزوّد والموديل يدويًا.",reread:"أعد القراءة من Pi",skipped:(n:number)=>`تُرك ${n} من أسطر القائمة لأن صيغتها غير مقبولة.`,choose:"اختر مزوّدًا",notListed:"غير موجود في قائمة Pi",piDefault:(model:string)=>`افتراضي Pi (${model})`,defaultMissing:(model:string)=>`افتراضي Pi لهذا المزوّد (${model}) غير موجود في قائمته؛ اختر موديلًا.`,noDefault:"ليس لهذا المزوّد موديل افتراضي في Pi 1.5.2؛ اختر موديلًا.",defaultNote:"«افتراضي Pi» يمرّر موديل Pi 1.5.2 الافتراضي لهذا المزوّد صراحةً، ويظهر اسمه في نافذة التأكيد ويُربط به رمزها."},
 en:{title:"Pi — experimental",enable:"Use Pi for classification (off by default)",terms:"Using providers' subscriptions through Pi is subject to each provider's terms; the responsibility lies with the user.",help:"Pi runs after Claude in automatic mode once you approve it explicitly in that run, or when you choose it as the agent. Pi does not list its tools when a session starts, so Kinana uses the alternative gate you approved: a reviewed version only (1.5.2), flags that turn off tools, extensions, skills, context files, the session and start-up network, and an immediate stop on any tool or unknown event. Kinana never reads Pi's tokens or auth.json.",provider:"Provider (required when Pi is on)",model:"Model (optional)",modelManual:"Model (optional: leave empty for Pi's default model for this provider)",thinking:"Thinking level",account:"Account type",default:"Pi default",unspecified:"Not specified",subscription:"Subscription",apiKey:"API key",cache:"Note: if cache warming (cacheWarming) is on in Pi's settings, Pi may send extra model requests that Kinana does not control.",
  reading:"Reading providers and models from Pi…",read:(version:string,count:number)=>`From Pi ${version}: ${count} ready provider${count===1?"":"s"}. The list comes from Pi itself, through a read command that sends no text and calls no model.`,empty:"Pi lists no ready provider. Sign in to Pi or set a key, then read again, or type the provider and model.",failed:"Pi's list could not be read: ",manual:"Type the provider and model instead.",reread:"Read again from Pi",skipped:(n:number)=>`${n} list line${n===1?" was":"s were"} left out because their format is not accepted.`,choose:"Choose a provider",notListed:"not in Pi's list",piDefault:(model:string)=>`Pi default (${model})`,defaultMissing:(model:string)=>`Pi's default for this provider (${model}) is not in its list; choose a model.`,noDefault:"Pi 1.5.2 has no default model for this provider; choose a model.",defaultNote:"“Pi default” passes Pi 1.5.2's default model for this provider explicitly; the confirmation shows its name and its token binds it."},
};
const listReasons:Record<string,[string,string]>={
 AGENT_EXECUTABLE_NOT_FOUND:["لم يُعثر على أمر pi موثوق خارج المخزن ومجلدات المصادر.","No trusted pi command was found outside the vault and source folders."],
 AGENT_EXECUTABLE_UNTRUSTED:["أمر pi الموجود غير موثوق (مكانه أو صلاحياته)، فلم يُشغَّل.","The pi command found is not trusted (its location or permissions), so it was not started."],
 AGENT_TIMEOUT:["لم تكتمل قراءة القائمة من Pi في المهلة، فأُوقف.","Pi did not finish listing in time and was stopped."],
 AGENT_EXIT_FAILED:["انتهى Pi بخطأ أثناء القراءة، وحُجبت تفاصيله حماية لبيانات الدخول.","Pi ended with an error while listing; its details were withheld to protect sign-in data."],
 PI_MODELS_UNREADABLE:["ردّ Pi ليس بصيغة القائمة المراجَعة.","Pi's answer is not in the reviewed list format."],
};
function listReason(code:string,locale:string){const own=listReasons[code];return own?own[locale==="ar"?0:1]:reasonText(code,locale);}
type ListState={state:"idle"|"loading"}|{state:"ok";list:PiModelList}|{state:"error";error:CoreError};
export function PiSettingsSection({t,value,onChange}:{t:Text;value:Settings;onChange:(next:Settings)=>void}) {
 const w=words[t.locale as "ar"|"en"];
 const set=(patch:Partial<Settings>)=>onChange({...value,...patch});
 const [list,setList]=useState<ListState>({state:"idle"});
 const read=async()=>{setList({state:"loading"});try{setList({state:"ok",list:await api.piModels()});}catch(e){setList({state:"error",error:asError(e)});}};
 // Owner, 2026-11-24: the providers and models come from Pi itself, read
 // when Pi is turned on (and again on request), never from Pi's files.
 useEffect(()=>{if(value.enabled&&list.state==="idle")void read();},[value.enabled]); // eslint-disable-line react-hooks/exhaustive-deps
 const providers=list.state==="ok"?list.list.providers:[];
 const fromList=list.state==="ok"&&providers.length>0;
 const provider=providers.find(p=>p.id===value.provider);
 const chooseProvider=(id:string)=>{const next=providers.find(p=>p.id===id);if(!next)return;set({provider:id,model:next.default_model&&next.default_listed?"":next.models[0]?.id??""});};
 const providerOptions=[...(value.provider?[]:[{value:NONE,label:w.choose,disabled:true}]),...providers.map(p=>({value:p.id,label:p.id})),...(value.provider&&!provider?[{value:value.provider,label:`${value.provider} (${w.notListed})`}]:[])];
 const modelOptions=provider?[...(provider.default_model?[{value:PI_DEFAULT,label:w.piDefault(provider.default_model),disabled:!provider.default_listed}]:[]),...provider.models.map(m=>({value:m.id,label:m.id})),...(value.model&&!provider.models.some(m=>m.id===value.model)?[{value:value.model,label:`${value.model} (${w.notListed})`}]:[])]:[];
 const status=list.state==="loading"?<p>{w.reading}</p>
  :list.state==="error"?<p className="classification-warning">{w.failed}<span data-testid="pi-models-reason">{listReason(list.error.reason_code||list.error.code,t.locale)}</span> {w.manual}</p>
  :list.state==="ok"?<p>{providers.length?w.read(list.list.version,providers.length):w.empty}{list.list.skipped_rows>0&&<> {w.skipped(list.list.skipped_rows)}</>}</p>:null;
 return <section className="settings-section pi-settings" data-testid="pi-settings"><h2>{w.title}</h2>
  <label className="checkbox-label"><Checkbox data-testid="pi-enabled" checked={value.enabled} onCheckedChange={v=>set({enabled:v===true})}/>{w.enable}</label>
  <p className="settings-help">{w.help}</p>
  <p className="settings-help" role="note" data-testid="pi-terms">{w.terms}</p>
  {value.enabled&&<div className="pi-fields">
   <div className="pi-models-status settings-help" role="status" aria-live="polite" data-testid="pi-models-status" data-state={list.state==="ok"&&!fromList?"empty":list.state} data-code={list.state==="error"?list.error.reason_code||list.error.code:undefined}>{status}{list.state!=="loading"&&<Button size="sm" variant="outline" data-testid="pi-models-reread" onClick={()=>void read()}><RefreshCw/>{w.reread}</Button>}</div>
   {fromList?<>
    <div className="settings-label"><span>{w.provider}</span><Choice testId="pi-provider-choice" label={w.provider} value={value.provider||NONE} options={providerOptions} onChange={chooseProvider}/></div>
    {provider&&<div className="settings-label"><span>{w.model}</span><Choice testId="pi-model-choice" label={w.model} value={value.model||PI_DEFAULT} options={modelOptions} onChange={v=>set({model:v===PI_DEFAULT?"":v})}/></div>}
    {provider&&!provider.default_model&&<p className="settings-help classification-warning" data-testid="pi-no-default">{w.noDefault}</p>}
    {provider?.default_model&&!provider.default_listed&&<p className="settings-help classification-warning" data-testid="pi-default-missing">{w.defaultMissing(provider.default_model)}</p>}
   </>:list.state!=="loading"&&<>
    <label className="settings-label">{w.provider}<Input dir="ltr" data-testid="pi-provider" value={value.provider} onChange={e=>set({provider:e.target.value.trim()})}/></label>
    <label className="settings-label">{w.modelManual}<Input dir="ltr" data-testid="pi-model" value={value.model} onChange={e=>set({model:e.target.value.trim()})}/></label>
   </>}
   <p className="settings-help" data-testid="pi-default-note">{w.defaultNote}</p>
   <div className="settings-label"><span>{w.thinking}</span><Choice testId="pi-thinking" label={w.thinking} value={value.thinking||"default"} options={[{value:"default",label:w.default},...["off","minimal","low","medium","high","xhigh","max"].map(v=>({value:v,label:v}))]} onChange={v=>set({thinking:v==="default"?"":v})}/></div>
   <div className="settings-label"><span>{w.account}</span><Choice testId="pi-account" label={w.account} value={value.account||"unspecified"} options={[{value:"unspecified",label:w.unspecified},{value:"subscription",label:w.subscription},{value:"api_key",label:w.apiKey}]} onChange={v=>set({account:v==="unspecified"?"":v})}/></div>
   <p className="settings-help" role="note">{w.cache}</p>
  </div>}
 </section>;
}
export const piNotes={
 ar:{subscription:"Pi باشتراك: يستهلك من حدود خطتك عند المزوّد.",api_key:"Pi بمفتاح API: يُحاسب بالتوكن عند المزوّد. التقدير بالتوكنات فقط.",unspecified:"Pi: نوع الحساب غير محدد في الإعدادات؛ قد يستهلك من حدود خطتك أو يُحاسب بالتوكن.",uncalibrated:"تقدير Pi غير معايَر: يحسب نص الطلب وحده دون ما يضيفه Pi والمزوّد.",cache:"قد يرسل Pi طلبات تسخين كاش إضافية إذا كان cacheWarming مفعّلًا في إعداداته."},
 en:{subscription:"Pi with a subscription: uses your plan's limits at the provider.",api_key:"Pi with an API key: billed per token by the provider. The estimate is in tokens only.",unspecified:"Pi: the account type is not set in the settings; it may use your plan's limits or be billed per token.",uncalibrated:"The Pi estimate is not calibrated: it counts the request text only, not what Pi and the provider add.",cache:"Pi may send extra cache-warming requests if cacheWarming is on in its settings."},
};
