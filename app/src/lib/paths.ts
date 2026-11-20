export function displayPath(path:string,home:string,project?:string):string {
 const separator=path.includes("\\")?"\\":"/";
 function relative(base:string){const root=base.replace(/[\\/]+$/,"");return path===root?"":path.startsWith(root+separator)?path.slice(root.length+1):null;}
 if(project){const part=relative(project);if(part!==null)return part||".";}
 if(home){const part=relative(home);if(part!==null)return part?`~${separator}${part}`:"~";}
 return path;
}

// Same visible spelling as core names::visible. Escape untrusted text only at display time.
export function visibleName(value:string):string {
 const chars=Array.from(value);
 return chars.map((c,i)=>{
  if(c==="\\") return "\\\\";
  if(c===" " && i!==0 && i!==chars.length-1) return c;
  if(/[\p{Cc}\p{Cf}\p{Z}\p{Default_Ignorable_Code_Point}\u205f-\u206f\ufff0-\ufffb]/u.test(c))
   return `\\u{${c.codePointAt(0)!.toString(16)}}`;
  return c;
 }).join("");
}
