import AppKit
import Carbon
import CoreGraphics
import ApplicationServices
import Foundation
func string(_ source:TISInputSource,_ key:CFString)->String {guard let p=TISGetInputSourceProperty(source,key) else{return ""};return Unmanaged<CFString>.fromOpaque(p).takeUnretainedValue() as String}
func flag(_ source:TISInputSource,_ key:CFString)->Bool {guard let p=TISGetInputSourceProperty(source,key) else{return false};return CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(p).takeUnretainedValue())}
let sources=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
let current=TISCopyCurrentKeyboardInputSource().takeRetainedValue()
let original=string(current,kTISPropertyInputSourceID)
let enabledBefore=Set(sources.filter{flag($0,kTISPropertyInputSourceIsEnabled)}.map{string($0,kTISPropertyInputSourceID)})
var report:[String:Any]=["source_before":original,"post_event_access":CGPreflightPostEventAccess()]
let appPID=pid_t(CommandLine.arguments[1])!
report["target_pid"]=appPID
report["foreground_guarded_os_events"]=true
report["frontmost_pid_at_start"]=NSWorkspace.shared.frontmostApplication?.processIdentifier
func perform() throws {
 guard CGPreflightPostEventAccess() else {throw NSError(domain:"Event access unavailable",code:1)}
 guard let source=sources.first(where:{string($0,kTISPropertyInputSourceID)=="com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"}) else {throw NSError(domain:"Japanese Romaji IME unavailable",code:2)}
 if let parent=sources.first(where:{string($0,kTISPropertyInputSourceID)=="com.apple.inputmethod.Kotoeri.RomajiTyping"}) {report["enable_parent_status"]=TISEnableInputSource(parent)}
 report["enable_status"]=TISEnableInputSource(source)
 RunLoop.current.run(until:Date().addingTimeInterval(2))
 let targetApp=AXUIElementCreateApplication(appPID)
 report["ax_trusted"]=AXIsProcessTrusted()
 report["ax_frontmost_status"]=AXUIElementSetAttributeValue(targetApp,kAXFrontmostAttribute as CFString,kCFBooleanTrue).rawValue
 var windows:CFTypeRef?
 if AXUIElementCopyAttributeValue(targetApp,kAXWindowsAttribute as CFString,&windows) == .success,let windows=windows as? [AXUIElement],let window=windows.first {report["ax_raise_status"]=AXUIElementPerformAction(window,kAXRaiseAction as CFString).rawValue}
 NSRunningApplication(processIdentifier:appPID)?.activate(options:[.activateAllWindows])
 RunLoop.current.run(until:Date().addingTimeInterval(0.5))
 if CommandLine.arguments.count>=5,let windows=CGWindowListCopyWindowInfo([.optionOnScreenOnly],kCGNullWindowID) as? [[String:Any]],let window=windows.first(where:{($0[kCGWindowOwnerPID as String] as? Int)==Int(appPID)&&($0[kCGWindowLayer as String] as? Int)==0}),let b=window[kCGWindowBounds as String] as? [String:Double],let x=b["X"],let y=b["Y"],let height=b["Height"] {
     let point=CGPoint(x:x+Double(CommandLine.arguments[2])!,y:y+height-Double(CommandLine.arguments[4])!+Double(CommandLine.arguments[3])!)
     report["input_click_point"]=["x":point.x,"y":point.y];report["window_bounds"]=b;report["dom_coordinates"]=Array(CommandLine.arguments.dropFirst(2))
     CGEvent(mouseEventSource:nil,mouseType:.leftMouseDown,mouseCursorPosition:point,mouseButton:.left)?.post(tap:.cghidEventTap)
     CGEvent(mouseEventSource:nil,mouseType:.leftMouseUp,mouseCursorPosition:point,mouseButton:.left)?.post(tap:.cghidEventTap)
     RunLoop.current.run(until:Date().addingTimeInterval(0.2))
     report["native_input_click"]=true
 }
 let selected=TISSelectInputSource(source);report["select_status"]=selected
 guard selected==noErr else {throw NSError(domain:"IME selection failed",code:Int(selected))}
 RunLoop.current.run(until:Date().addingTimeInterval(1))
 report["key_sources"] = [String]()
 report["source_during"]=string(TISCopyCurrentKeyboardInputSource().takeRetainedValue(),kTISPropertyInputSourceID)
 for key:CGKeyCode in [45,34,4,31,45,5,31,49,36,36,48] {
  // Global hardware events are posted only while the verified test app owns foreground.
  guard let target=NSRunningApplication(processIdentifier:appPID), !target.isTerminated, target.executableURL?.lastPathComponent=="kinana-desktop" else {throw NSError(domain:"Target test app unavailable; no keys posted",code:3)}
  report["key_sources"]=(report["key_sources"] as! [String])+["\(key):"+string(TISCopyCurrentKeyboardInputSource().takeRetainedValue(),kTISPropertyInputSourceID)]
  report["frontmost_pid_during"]=NSWorkspace.shared.frontmostApplication?.processIdentifier
  guard NSWorkspace.shared.frontmostApplication?.processIdentifier==appPID else {throw NSError(domain:"Target app lost foreground; no global keys posted",code:3)}
  let down=CGEvent(keyboardEventSource:CGEventSource(stateID:.hidSystemState),virtualKey:key,keyDown:true)!;down.flags=[];down.post(tap:.cghidEventTap)
  let up=CGEvent(keyboardEventSource:CGEventSource(stateID:.hidSystemState),virtualKey:key,keyDown:false)!;up.flags=[];up.post(tap:.cghidEventTap)
  RunLoop.current.run(until:Date().addingTimeInterval(0.12))
 }
 // Finish composition with a real mouse click in the target window's empty header.
 if let windows=CGWindowListCopyWindowInfo([.optionOnScreenOnly],kCGNullWindowID) as? [[String:Any]],let window=windows.first(where:{($0[kCGWindowOwnerPID as String] as? Int)==Int(appPID)&&($0[kCGWindowLayer as String] as? Int)==0}),let bounds=window[kCGWindowBounds as String] as? [String:Double],let x=bounds["X"],let y=bounds["Y"],let width=bounds["Width"] {
     guard NSWorkspace.shared.frontmostApplication?.processIdentifier==appPID else {throw NSError(domain:"Target lost foreground before native commit click",code:4)}
     let point=CGPoint(x:x+width/2,y:y+60)
     CGEvent(mouseEventSource:nil,mouseType:.leftMouseDown,mouseCursorPosition:point,mouseButton:.left)?.post(tap:.cghidEventTap)
     CGEvent(mouseEventSource:nil,mouseType:.leftMouseUp,mouseCursorPosition:point,mouseButton:.left)?.post(tap:.cghidEventTap)
     report["native_commit_click"]=true
 }
 RunLoop.current.run(until:Date().addingTimeInterval(0.4))
 report["typed_keys"]="nihongo + Space + Return + Return + Tab"
}
do {try perform();report["success"]=true}catch{report["success"]=false;report["error"]=String(describing:error)}
report["restore_status"]=TISSelectInputSource(current)
let afterSources=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
for source in afterSources where flag(source,kTISPropertyInputSourceIsEnabled) && !enabledBefore.contains(string(source,kTISPropertyInputSourceID)) {TISDisableInputSource(source)}
let finalSources=TISCreateInputSourceList(nil,true).takeRetainedValue() as! [TISInputSource]
let enabledAfter=Set(finalSources.filter{flag($0,kTISPropertyInputSourceIsEnabled)}.map{string($0,kTISPropertyInputSourceID)})
report["source_after"]=string(TISCopyCurrentKeyboardInputSource().takeRetainedValue(),kTISPropertyInputSourceID)
report["enabled_sources_restored"]=enabledBefore==enabledAfter
print(String(data:try JSONSerialization.data(withJSONObject:report,options:[.sortedKeys]),encoding:.utf8)!)
if report["success"] as? Bool != true || original != report["source_after"] as? String || enabledBefore != enabledAfter {exit(1)}
