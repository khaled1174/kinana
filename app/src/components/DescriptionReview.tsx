import { useMemo, useState } from "react";
import { Button } from "./ui/button";
import { descriptionDiff } from "../lib/presentation";
import type { Text } from "../i18n";
export function DescriptionReview({original,proposed,t}:{original:string;proposed:string;t:Text}) {
  const [show,setShow]=useState(false);
  const diff = useMemo(() => descriptionDiff(original, proposed), [original, proposed]);
  return <div className="description-comparison" data-testid="description-comparison">
    <Button size="sm" variant="ghost" data-testid="toggle-description-diff" aria-pressed={show} onClick={()=>setShow(!show)}>{t.locale==="ar"?(show?"إخفاء الفروق":"إظهار الفروق"):(show?"Hide differences":"Show differences")}</Button>
    {show&&<p className="diff-legend"><span className="removed-key">{t.removedWords}</span><span className="added-key">{t.addedWords}</span></p>}
    <section><h3>{t.original}</h3><p dir="auto" data-testid="original-description">{!show?original:diff.original.map((part,i)=>part.changed?<del key={i}>{part.text}</del>:<span key={i}>{part.text}</span>)}</p></section>
    <section><h3>{t.suggested}</h3><p dir="auto" data-testid="proposed-description">{!show?proposed:diff.proposed.map((part,i)=>part.changed?<ins key={i}>{part.text}</ins>:<span key={i}>{part.text}</span>)}</p></section>
  </div>;
}
