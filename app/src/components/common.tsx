import { AlertCircle, Inbox } from "lucide-react";
import { Alert, AlertTitle, AlertDescription } from "./ui/alert";
import { Empty, EmptyHeader, EmptyTitle, EmptyDescription, EmptyMedia } from "./ui/empty";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectGroup, SelectItem } from "./ui/select";
import type { CoreError } from "../bridge";
import {errorText} from "../reasons";
import type { Text } from "../i18n";
export function ErrorNotice({error,t}:{error:CoreError;t:Text}) {return <Alert variant="destructive" data-testid="error"><AlertCircle/><AlertTitle>{t.error} · <bdi>{error.code}</bdi></AlertTitle><AlertDescription><p dir="auto">{errorText(error,t.locale)}</p></AlertDescription></Alert>;}
export function EmptyNotice({title,body,children}:{title:string;body?:string;children?:React.ReactNode}) {return <Empty><EmptyHeader><EmptyMedia variant="icon"><Inbox/></EmptyMedia><EmptyTitle>{title}</EmptyTitle>{body&&<EmptyDescription>{body}</EmptyDescription>}</EmptyHeader>{children}</Empty>;}
export function Choice({label,value,options,onChange,testId}:{label:string;value:string;options:{value:string;label:string;disabled?:boolean}[];onChange:(value:string)=>void;testId?:string}) {
  return <Select value={value} onValueChange={onChange}><SelectTrigger aria-label={label} data-testid={testId}><SelectValue/></SelectTrigger><SelectContent><SelectGroup>{options.map(option=><SelectItem key={option.value} value={option.value} disabled={option.disabled} data-testid={testId?`${testId}-${option.value}`:undefined}>{option.label}</SelectItem>)}</SelectGroup></SelectContent></Select>;
}
export function labelFor(t:Text,value:string) {return value in t ? t[value as keyof Text] : value;}
