import {reasonText} from "../reasons";
export function CodeNotices({codes=[],locale}:{codes?:string[];locale:string}) {return <>{codes.map((code,i)=><p className="notice" key={`${code}-${i}`} data-reason-code={code}>{reasonText(code,locale)}</p>)}</>;}
