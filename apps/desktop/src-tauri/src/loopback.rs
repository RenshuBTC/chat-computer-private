//! On a Mac, the app's own page comes from a small server only this computer can reach (127.0.0.1): a Mac's web
//! view doesn't count Tauri's own scheme as secure, and a page that isn't gets no WebGPU (Heart), no microphone
//! and no browser cache. It serves the files bundled into the app, and nothing else; it answers only requests
//! addressed to it by that name (not another name pointed at this computer).
use tauri::{AssetResolver, Runtime};

/// The same port each time where it's free, so what the page keeps (in the browser's storage, by address) is
/// there next time.
pub const PORTS: [u16; 3] = [42851, 42852, 42853];

pub fn serve<R: Runtime>(assets: AssetResolver<R>) -> Result<u16, String> {
    for port in PORTS {
        let Ok(server) = tiny_http::Server::http(("127.0.0.1", port)) else { continue };
        let me = format!("127.0.0.1:{port}");
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let host_ok = req.headers().iter().any(|h| h.field.equiv("Host") && h.value.as_str() == me);
                let path = req.url().split(['?', '#']).next().unwrap_or("/").to_string();
                let path = if path == "/" { "/index.html".to_string() } else { path };
                let asset = if host_ok && *req.method() == tiny_http::Method::Get { assets.get(path) } else { None };
                let Some(asset) = asset else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                    continue;
                };
                let mut resp = tiny_http::Response::from_data(asset.bytes);
                let mut headers = vec![
                    ("Content-Type".to_string(), asset.mime_type),
                    ("Cache-Control".to_string(), "no-cache".to_string()),
                    ("X-Content-Type-Options".to_string(), "nosniff".to_string()),
                    ("Cross-Origin-Resource-Policy".to_string(), "same-origin".to_string()),
                ];
                if let Some(csp) = asset.csp_header {
                    headers.push(("Content-Security-Policy".to_string(), csp));
                }
                for (k, v) in headers {
                    if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                        resp.add_header(h);
                    }
                }
                let _ = req.respond(resp);
            }
        });
        return Ok(port);
    }
    Err("Chat Computer couldn't start its page: ports 42851 to 42853 are all taken.".into())
}
