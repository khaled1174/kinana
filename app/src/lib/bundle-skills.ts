import type {Catalog} from "../bridge";

export async function loadBundleSkills(vault:string, catalog:()=>Promise<Catalog>) {
 const result=await catalog();
 if(result.vault!==vault)throw {code:"VAULT_CHANGED",message:"Reopen the bundle in the selected vault."};
 return result.items.map(item=>item.skill);
}
