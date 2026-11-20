// Own one temporary OS input-method session across the two native text fields.
import Carbon
import Foundation
func id(_ s:TISInputSource)->String {Unmanaged<CFString>.fromOpaque(TISGetInputSourceProperty(s,kTISPropertyInputSourceID)!).takeUnretainedValue() as String}
func enabled(_ s:TISInputSource)->Bool {CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(TISGetInputSourceProperty(s,kTISPropertyInputSourceIsEnabled)!).takeUnretainedValue())}
let sources=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
let url=URL(fileURLWithPath:CommandLine.arguments[2])
if CommandLine.arguments[1]=="prepare" {
 let state:[String:Any]=["selected":id(TISCopyCurrentKeyboardInputSource().takeRetainedValue()),"enabled":sources.filter(enabled).map(id)]
 try JSONSerialization.data(withJSONObject:state,options:[.sortedKeys]).write(to:url,options:.atomic)
 for s in sources where ["com.apple.inputmethod.Kotoeri.RomajiTyping","com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"].contains(id(s)){guard TISEnableInputSource(s)==noErr else{exit(1)}}
 RunLoop.current.run(until:Date().addingTimeInterval(2))
 guard let source=sources.first(where:{id($0)=="com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"}),TISSelectInputSource(source)==noErr else{exit(1)}
 RunLoop.current.run(until:Date().addingTimeInterval(1))
 print(String(data:try JSONSerialization.data(withJSONObject:state),encoding:.utf8)!)
} else {
 let state=try JSONSerialization.jsonObject(with:Data(contentsOf:url)) as! [String:Any]
 guard let source=sources.first(where:{id($0)==state["selected"] as? String}),TISSelectInputSource(source)==noErr else{exit(1)}
 let before=Set(state["enabled"] as! [String]);for s in sources where enabled(s) && !before.contains(id(s)){TISDisableInputSource(s)}
 let final=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
 let after=Set(final.filter(enabled).map(id));let selected=id(TISCopyCurrentKeyboardInputSource().takeRetainedValue())
 let restored=before==after && selected==state["selected"] as? String
 print(String(data:try JSONSerialization.data(withJSONObject:["restored":restored,"selected_after":selected,"enabled_sources_restored":before==after]),encoding:.utf8)!)
 if !restored{exit(1)}
}
