import AppKit
import WebKit
import Foundation
let html=try String(contentsOfFile:CommandLine.arguments[1],encoding:.utf8)
let application=NSApplication.shared
application.setActivationPolicy(.accessory)
let configuration=WKWebViewConfiguration()
configuration.websiteDataStore = .nonPersistent()
let view=WKWebView(frame:NSRect(x:0,y:0,width:700,height:500),configuration:configuration)
let window=NSWindow(contentRect:view.frame,styleMask:[.titled],backing:.buffered,defer:false)
window.contentView=view
window.title="Kinana CSP positive control (test)"
window.makeKeyAndOrderFront(nil)
application.activate(ignoringOtherApps:true)
view.loadHTMLString(html,baseURL:URL(string:CommandLine.arguments[2])!)
DispatchQueue.main.asyncAfter(deadline:.now()+2){view.evaluateJavaScript("JSON.stringify({inline:window.__kinanaInlineExecuted===true,dom:!!document.querySelector('#raw-csp-payload')})"){value,error in
 print(value as? String ?? "{\"error\":true}");fflush(stdout);exit(error==nil ? 0:1)
}}
DispatchQueue.main.asyncAfter(deadline:.now()+10){print("{\"error\":\"positive control timed out\"}");fflush(stdout);exit(2)}
application.run()
