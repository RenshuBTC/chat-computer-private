// Chat Computer for iPhone and iPad: chatcomputer.ai, full screen, in an app of its own (Safari's engine, WKWebView).
// The page is the website's, so it's always the newest. Other sites it opens (in a new tab on the website) open in
// Safari's own view over it, with Done to come back; the microphone is asked for when talking is turned on.
import SafariServices
import SwiftUI
import WebKit

let home = URL(string: "https://chatcomputer.ai/")!
/// The screen's dark (theme.css's ground), around the page.
let ground = Color(red: 0x15 / 255, green: 0x15 / 255, blue: 0x15 / 255)

@main
struct ChatComputerApp: App {
    var body: some Scene {
        WindowGroup {
            ZStack {
                ground.ignoresSafeArea()            // (behind the status bar and the home indicator too)
                Page()
            }
            .preferredColorScheme(.dark)            // (the status bar's light on it)
        }
    }
}

struct Page: UIViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []
        // Sites treat it as the Safari it is (some turn away a browser they don't know).
        config.applicationNameForUserAgent = "Version/26.0 Mobile/15E148 Safari/604.1"
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = UIColor(ground)
        view.scrollView.backgroundColor = UIColor(ground)
        view.scrollView.contentInsetAdjustmentBehavior = .never     // (it's inside the safe area already)
        view.allowsBackForwardNavigationGestures = false
        #if DEBUG
        view.isInspectable = true
        #endif
        view.load(URLRequest(url: home))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        /// chatcomputer.ai stays here; another site opens in Safari's own view over it.
        func webView(_ view: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            guard let url = action.request.url, action.targetFrame?.isMainFrame ?? true else {
                decisionHandler(.allow)
                return
            }
            if url.host == home.host || url.scheme == "about" || url.scheme == "blob" {
                decisionHandler(.allow)
            } else {
                decisionHandler(.cancel)
                open(url, from: view)
            }
        }

        /// A new tab or window (window.open, a link meant for one): Safari's view, over the page.
        func webView(_ view: WKWebView, createWebViewWith config: WKWebViewConfiguration,
                     for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = action.request.url { open(url, from: view) }
            return nil
        }

        /// The microphone, for talking: the system asks once, as it does for Safari.
        func webView(_ view: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin,
                     initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType,
                     decisionHandler: @escaping (WKPermissionDecision) -> Void) {
            decisionHandler(origin.host == home.host ? .prompt : .deny)
        }

        /// The page's own process stopped (the phone was short of memory): load it again.
        func webViewWebContentProcessDidTerminate(_ view: WKWebView) {
            view.load(URLRequest(url: home))
        }

        private func open(_ url: URL, from view: WKWebView) {
            guard url.scheme == "https" || url.scheme == "http" else {
                UIApplication.shared.open(url)          // mailto:, tel:, an app's own link
                return
            }
            var top = view.window?.rootViewController
            while let shown = top?.presentedViewController { top = shown }
            let safari = SFSafariViewController(url: url)
            safari.preferredControlTintColor = .black
            top?.present(safari, animated: true)
        }
    }
}
