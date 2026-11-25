import type {Audit} from "../bridge";
import type {Text} from "../i18n";
import {reasonText} from "../reasons";
import {PathView} from "./PathView";
import {auditRulesState} from "../lib/audit-rules";
import {auditGroups,placesLabel,type AuditGroup} from "../lib/audit-groups";

type Detail = Audit["details"][number];

// r5 (owner, 2026-11-24, item 4b): reasons grouped by rule and file, risky
// first, each group expandable to its lines.
const ruleLabels: Record<string, [string, string]> = {
  secrets: ["أسرار أو وصول لمتغيرات بيئة", "Secrets or environment variable access"],
  network: ["وصول للشبكة", "Network access"],
  destructive: ["أمر حذف أو تدمير", "Deleting or destructive command"],
  download_execution: ["تنزيل ثم تنفيذ", "Download then execution"],
  package_execution: ["تشغيل حزمة غير مقيّدة الإصدار", "Unpinned package execution"],
  instruction_override: ["تجاوز التعليمات أو إخفاء عن المستخدم", "Instruction override or concealment"],
  executable: ["ملف تنفيذي أو سكربت", "Executable or script file"],
  symlink: ["رابط رمزي", "Symbolic link"],
  hooks: ["hooks في البيانات الأمامية", "Hooks in frontmatter"],
};

// The two markers core writes in place of a literal value (audit/forms.rs).
const hiddenForms: Record<string, [string, string]> = {
  "literal secret value (hidden)": ["قيمة سرية حرفية (محجوبة)", "literal secret value (hidden)"],
  "private key block (hidden)": ["مفتاح خاص (محجوب)", "private key block (hidden)"],
};

function formText(form: string, ar: boolean) {
  return hiddenForms[form]?.[ar ? 0 : 1] ?? form;
}

function groupForms(group: AuditGroup) {
  const seen: string[] = [];
  for (const detail of group.details) for (const form of detail.forms ?? []) if (!seen.includes(form)) seen.push(form);
  return seen;
}

function Line({ d, t }: { d: Detail; t: Text }) {
  const ar = t.locale === "ar";
  return <article data-reason-code={d.reason_code} data-testid="audit-line">
    <p>
      <strong>{reasonText(d.reason_code, t.locale)}</strong>{" "}{ar ? "في" : "in"}{" "}
      {/* One left-to-right run, so Arabic text never reorders it into "3path:" (r4). */}
      <bdi dir="ltr" className="audit-location" data-testid="audit-location"><PathView path={d.path}/>{d.line !== null && <>:{d.line}{d.end_line !== null && d.end_line !== d.line && "–" + d.end_line}</>}</bdi>
      {" · "}{d.level === "risky" ? t.risky : t.review}
    </p>
    {/* r5 (owner, item 4a): the names and forms are shown; values never are. */}
    {!!d.forms?.length && <p className="audit-forms" data-testid="audit-forms">{ar ? "الصيغة" : "Form"}: {d.forms.map(form => <code key={form} dir="ltr" data-testid="audit-form">{formText(form, ar)}</code>)}</p>}
    {d.excerpt !== null ? <blockquote dir="auto">{d.excerpt}</blockquote> : d.line !== null &&
      <p className="notice">{ar ? "حُجب المقتطف لاحتمال احتوائه معلومات حساسة." : "Excerpt withheld because it may contain sensitive information."}</p>}
  </article>;
}

export function AuditDetails({audit,t}:{audit:Audit;t:Text}) {
 if(!audit.rules_version)return <p className="notice" data-testid="legacy-audit">{t.locale==="ar"?"هذه نتيجة فحص قديمة. أعد الفحص من الإعدادات لعرض القاعدة والسطر.":"This is a legacy audit. Refresh it in Settings to see rules and source lines."}</p>;
 const older=auditRulesState(audit.rules_version)==="older"&&<p className="notice" data-testid="older-rules-audit">{t.auditOlderRules}</p>;
 const ar=t.locale==="ar";
 const groups=auditGroups(audit.details);
 // An audit stored before r5 has no forms; a refresh adds them.
 const formsMissing=audit.details.some(d=>d.rule==="secrets")&&!audit.details.some(d=>d.forms?.length);
 return <div className="audit-details">
   {older}
   {formsMissing&&<p className="notice" data-testid="audit-forms-refresh">{ar?"لعرض أسماء المتغيرات وصيغها في أسباب الأسرار، أعد الفحص من الإعدادات ← الصيانة ← تحديث الفحص.":"To see variable names and forms in the secrets reasons, refresh the audit in Settings › Maintenance › Refresh audit."}</p>}
   {groups.length ? groups.map(group=>{
     const key=`${group.rule}\u0000${group.path}`;
     // r6 (owner, 2026-11-24): one place is one line, without a group
     // heading that repeats its rule, file and level.
     if(group.details.length===1)return <div key={key} className="audit-group audit-group-single" data-testid="audit-group" data-rule={group.rule} data-level={group.level} data-count={1} data-single="true"><Line d={group.details[0]} t={t}/></div>;
     const label=ruleLabels[group.rule]?.[ar?0:1]??group.rule;
     const forms=groupForms(group);
     const shownForms=forms.slice(0,3);
     const summary=<>
       <span className="audit-group-rule">{label}</span>{" · "}
       <span data-testid="audit-group-count" data-count={group.details.length}>{placesLabel(group.details.length,t.locale)}</span>{" · "}
       <bdi dir="ltr" className="audit-group-path"><PathView path={group.path}/></bdi>{" · "}
       <span className={group.level==="risky"?"audit-group-level risky":"audit-group-level"}>{group.level==="risky"?t.risky:t.review}</span>
       {shownForms.length>0&&<span className="audit-group-forms">{" · "}{shownForms.map(form=><code key={form} dir="ltr">{formText(form,ar)}</code>)}{forms.length>shownForms.length&&"…"}</span>}
     </>;
     return <details key={key} className="audit-group" data-testid="audit-group" data-rule={group.rule} data-level={group.level} data-count={group.details.length}>
       <summary data-testid="audit-group-summary">{summary}</summary>
       {group.details.map((d,i)=><Line key={i} d={d} t={t}/>)}
     </details>;
   }) : <p>{t.none}</p>}
 </div>;
}
