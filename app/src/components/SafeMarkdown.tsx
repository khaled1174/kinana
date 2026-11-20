import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

// Formatting only: automatic links become literal text; explicit links retain
// their destination as text. No raw-HTML processor or executable renderer.
type Node = {type:string;children?:Node[];value?:string;url?:string;position?:{start:{offset?:number};end:{offset?:number}}};
function literalAutolinks() {
  return (tree:Node,file:{value:unknown}) => {
    const source=String(file.value);
    function walk(node:Node) {
      if (!node.children) return;
      node.children=node.children.map(child=>{
        if (child.type === "link" && child.position) {
          const text=source.slice(child.position.start.offset,child.position.end.offset);
          if (!text.startsWith("[")) return {type:"text",value:text};
        }
        walk(child);return child;
      });
    }
    walk(tree);
  };
}
export function SafeMarkdown({text}:{text:string}) {
  return <div className="markdown" data-testid="safe-preview" dir="auto"><Markdown
    remarkPlugins={[remarkGfm,literalAutolinks]}
    skipHtml={false}
    urlTransform={url=>url}
    components={{
      a:({children,href})=><span className="literal-link">{children}{href && <> (<code>{href}</code>)</>}</span>,
      img:({alt})=><span className="image-alt">[{alt || "image"}]</span>,
      input:()=>null,
    }}
  >{text}</Markdown></div>;
}
