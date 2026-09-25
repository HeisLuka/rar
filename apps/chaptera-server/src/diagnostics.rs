use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{Html, Response},
    routing::get,
};
use serde::Serialize;

const REQUEST_ID_HEADER: &str = "x-request-id";
const DEFAULT_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RequestEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub level: &'static str,
    pub component: &'static str,
    pub event: &'static str,
    pub request_id: String,
    pub method: String,
    pub route: String,
    pub status: u16,
    pub elapsed_ms: u64,
}

#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<DiagnosticsInner>,
}

struct DiagnosticsInner {
    events: Mutex<VecDeque<RequestEvent>>,
    next_sequence: AtomicU64,
    capacity: usize,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl Diagnostics {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(DiagnosticsInner {
                events: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
                next_sequence: AtomicU64::new(1),
                capacity: capacity.max(1),
            }),
        }
    }

    fn push(&self, mut event: RequestEvent) {
        event.sequence = self.inner.next_sequence.fetch_add(1, Ordering::Relaxed);
        let mut events = self.inner.events.lock().expect("diagnostics mutex poisoned");
        if events.len() == self.inner.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    pub fn snapshot(&self) -> Vec<RequestEvent> {
        self.inner
            .events
            .lock()
            .expect("diagnostics mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

pub fn local_console_router(state: Diagnostics) -> Router {
    Router::new()
        .route("/__chaptera", get(console))
        .route("/__chaptera/events", get(events))
        .with_state(state)
}

async fn console() -> Html<&'static str> {
    Html(LOCAL_CONSOLE_HTML)
}

async fn events(State(state): State<Diagnostics>) -> Json<Vec<RequestEvent>> {
    Json(state.snapshot())
}

pub async fn record_requests(
    State(diagnostics): State<Diagnostics>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_owned();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "<unmatched>".to_owned());
    let skip_console_poll = matches!(
        request.uri().path(),
        "/__chaptera/events" | "/live" | "/ready" | "/version"
    );
    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let status = response.status();
    let request_id = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("missing")
        .to_owned();

    if !skip_console_poll {
        let event = RequestEvent {
            sequence: 0,
            timestamp_ms: now_ms(),
            level: level(status),
            component: "http",
            event: "request_complete",
            request_id,
            method,
            route,
            status: status.as_u16(),
            elapsed_ms,
        };
        match serde_json::to_string(&event) {
            Ok(line) => eprintln!("{line}"),
            Err(_) => eprintln!(
                "{{\"level\":\"error\",\"component\":\"diagnostics\",\"event\":\"serialize_failed\"}}"
            ),
        }
        diagnostics.push(event);
    }

    response
}

fn level(status: StatusCode) -> &'static str {
    if status.is_server_error() {
        "error"
    } else if status.is_client_error() {
        "warn"
    } else {
        "info"
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

const LOCAL_CONSOLE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Chaptera Local</title>
<style>
:root{font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color-scheme:dark;background:#111315;color:#eceff1}
*{box-sizing:border-box}body{margin:0}.wrap{max-width:1180px;margin:0 auto;padding:24px}.top{display:flex;gap:16px;align-items:center;justify-content:space-between;margin-bottom:20px}
h1{font-size:22px;margin:0}.sub{color:#9da7ae;font-size:13px}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:12px;margin-bottom:16px}
.card{background:#1b1f23;border:1px solid #2c3238;border-radius:12px;padding:14px}.card h2{font-size:12px;text-transform:uppercase;letter-spacing:.08em;color:#8c98a1;margin:0 0 8px}.value{font-size:18px;font-weight:650}.ok{color:#67d391}.bad{color:#ff7b72}.warn{color:#e3b341}
button{border:1px solid #39424a;background:#22282e;color:#eef2f4;border-radius:8px;padding:8px 12px;cursor:pointer}button:hover{background:#2a3138}
table{width:100%;border-collapse:collapse;font-size:12px}th,td{text-align:left;padding:8px;border-bottom:1px solid #2a3035;vertical-align:top}th{color:#8c98a1;font-weight:600}
.panel{background:#1b1f23;border:1px solid #2c3238;border-radius:12px;padding:14px;margin-top:12px;overflow:auto}.error{white-space:pre-wrap;color:#ff9c94;font-family:ui-monospace,SFMono-Regular,Consolas,monospace;font-size:12px}
.badge{display:inline-block;padding:2px 7px;border-radius:999px;background:#2b3137}.mono{font-family:ui-monospace,SFMono-Regular,Consolas,monospace}
</style>
</head>
<body>
<div class="wrap">
  <div class="top"><div><h1>Chaptera Local</h1><div class="sub">Local runtime console — status, readiness, build and recent HTTP events.</div></div><button id="copy">Copy diagnostics</button></div>
  <div class="grid">
    <div class="card"><h2>Process</h2><div id="live" class="value warn">Checking…</div></div>
    <div class="card"><h2>Readiness</h2><div id="ready" class="value warn">Checking…</div></div>
    <div class="card"><h2>Build</h2><div id="build" class="value mono">Checking…</div></div>
  </div>
  <div class="panel"><h2>Runtime components</h2><div id="components">Checking…</div></div>
  <div class="panel"><h2>Recent HTTP events</h2><table><thead><tr><th>Level</th><th>Status</th><th>Method</th><th>Route</th><th>ms</th><th>Request ID</th></tr></thead><tbody id="events"></tbody></table></div>
  <div class="panel"><h2>Browser/runtime errors</h2><div id="errors" class="error">None</div></div>
</div>
<script>
const state={live:null,ready:null,version:null,events:[],errors:[]};
const byId=id=>document.getElementById(id);
function err(message){state.errors.unshift(String(message));state.errors=state.errors.slice(0,20);byId("errors").textContent=state.errors.join("\n")||"None"}
window.addEventListener("error",event=>err("browser: "+event.message));
window.addEventListener("unhandledrejection",event=>err("promise: "+((event.reason&&event.reason.stack)||event.reason||"unknown")));
async function json(path){
  const response=await fetch(path,{cache:"no-store"});
  const body=await response.json().catch(()=>({}));
  if(!response.ok&&path!=="/ready") throw new Error(path+" -> "+response.status+" "+JSON.stringify(body));
  return {status:response.status,body:body};
}
function escapeHtml(value){return String(value).replace(/[&<>"']/g,function(ch){return {"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}[ch]})}
function render(){
  byId("live").textContent=state.live&&state.live.status===200?"LIVE":"DOWN";
  byId("live").className="value "+(state.live&&state.live.status===200?"ok":"bad");
  const isReady=Boolean(state.ready&&state.ready.body&&state.ready.body.ready===true);
  byId("ready").textContent=isReady?"READY":"NOT READY";
  byId("ready").className="value "+(isReady?"ok":"bad");
  byId("build").textContent=(state.version&&state.version.body&&(state.version.body.git_sha||state.version.body.version))||"unknown";
  const components=(state.ready&&state.ready.body&&state.ready.body.components)||{};
  byId("components").innerHTML=Object.entries(components).map(function(pair){
    const name=pair[0],value=pair[1];
    return '<div style="margin:5px 0"><span class="badge '+(value.ready?"ok":"bad")+'">'+(value.ready?"ready":"blocked")+'</span> <b>'+escapeHtml(name)+'</b>'+(value.code?' — <span class="mono">'+escapeHtml(value.code)+'</span>':'')+(value.message?' — '+escapeHtml(value.message):'')+'</div>';
  }).join("")||"No component report";
  byId("events").innerHTML=[...state.events].reverse().map(function(e){
    const cls=e.level==="error"?"bad":(e.level==="warn"?"warn":"");
    return '<tr><td class="'+cls+'">'+escapeHtml(e.level)+'</td><td>'+e.status+'</td><td>'+escapeHtml(e.method)+'</td><td class="mono">'+escapeHtml(e.route)+'</td><td>'+e.elapsed_ms+'</td><td class="mono">'+escapeHtml(e.request_id)+'</td></tr>';
  }).join("");
}
async function refresh(){
  try{
    const values=await Promise.all([json("/live"),json("/ready"),json("/version"),json("/__chaptera/events")]);
    state.live=values[0];state.ready=values[1];state.version=values[2];state.events=values[3].body||[];render();
  }catch(error){err(error.stack||error);render()}
}
byId("copy").addEventListener("click",async function(){
  const payload={captured_at:new Date().toISOString(),live:state.live,ready:state.ready,version:state.version,events:state.events,errors:state.errors};
  try{await navigator.clipboard.writeText(JSON.stringify(payload,null,2));byId("copy").textContent="Copied";setTimeout(function(){byId("copy").textContent="Copy diagnostics"},1200)}catch(error){err(error)}
});
refresh();setInterval(refresh,2000);
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn local_console_routes_are_available_when_router_is_mounted() {
        let response = local_console_router(Diagnostics::new(8))
            .oneshot(
                Request::builder()
                    .uri("/__chaptera")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn ring_buffer_is_bounded() {
        let diagnostics = Diagnostics::new(2);
        for status in [200_u16, 400, 500] {
            diagnostics.push(RequestEvent {
                sequence: 0,
                timestamp_ms: 1,
                level: "info",
                component: "http",
                event: "request_complete",
                request_id: "request".to_owned(),
                method: "GET".to_owned(),
                route: "/test".to_owned(),
                status,
                elapsed_ms: 1,
            });
        }
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].status, 400);
        assert_eq!(snapshot[1].status, 500);
    }
}
