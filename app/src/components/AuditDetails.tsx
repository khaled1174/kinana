import type {Audit} from "../bridge";
import type {Text} from "../i18n";
import {reasonText} from "../reasons";
import {PathView} from "./PathView";
export function AuditDetails({audit,t}:{audit:Audit;t:Text}) {
 if(!audit.rules_version)return <p className="notice" data-testid="legacy-audit">{t.locale==="ar"?"هذه نتيجة فحص قديمة. أعد الفحص من الإعدادات لعرض القاعدة والسطر.":"This is a legacy audit. Refresh it in Settings to see rules and source lines."}</p>;
 return <div className="audit-details">{audit.details.length?audit.details.map((d,i)=><article key={i} data-reason-code={d.reason_code}><strong>{reasonText(d.reason_code,t.locale)}</strong><p><PathView path={d.path}/>{d.line!==null&&<span>:{d.line}{d.end_line!==d.line&&`–${d.end_line}`}</span>} · {d.level==="risky"?t.risky:t.review}</p>{d.excerpt!==null?<blockquote dir="auto">{d.excerpt}</blockquote>:d.line!==null&&<p className="notice">{t.locale==="ar"?"حُجب المقتطف لاحتمال احتوائه معلومات حساسة.":"Excerpt withheld because it may contain sensitive information."}</p>}</article>):<p>{t.none}</p>}</div>;
}
