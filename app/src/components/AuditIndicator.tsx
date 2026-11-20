import type { Text } from "../i18n";
import { labelFor } from "./common";
export function AuditIndicator({level,t,label=false}:{level:string;t:Text;label?:boolean}) {
  const explanation = labelFor(t, `audit_${level}`);
  return <span className="audit-indicator" role="img" aria-label={explanation} title={explanation} data-audit={level}>
    {level!=="clean"&&<span className={`audit-dot audit-${level}`} aria-hidden="true"/>}{label&&<span>{t.audit}: {labelFor(t,level)}</span>}
  </span>;
}
