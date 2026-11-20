import {createContext,useContext} from "react";
import {displayPath,visibleName} from "../lib/paths";
export const PathContext=createContext({home:""});
export function PathView({path,project,className="",tail=false}:{path:string;project?:string;className?:string;tail?:boolean}) {const {home}=useContext(PathContext);const text=visibleName(displayPath(path,home,project));
 // A tail view keeps the end of a long path, where folders that differ usually are, and shortens its start.
 if(tail)return <span className={`path-tail ${className}`} title={visibleName(path)}><bdi dir="ltr" data-full-path={path}>{text}</bdi></span>;
 return <bdi dir="ltr" title={visibleName(path)} className={`path-view ${className}`} data-full-path={path}>{text}</bdi>;}
