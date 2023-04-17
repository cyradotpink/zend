mod peer_api;
mod room_api;
mod websocket;
mod websocket_api;
mod websocket_api_handlers;

use worker as w;

#[w::event(fetch)]
pub async fn fetch(req: w::Request, env: w::Env, _ctx: w::Context) -> w::Result<w::Response> {
    if req.headers().get("Upgrade")? == Some("websocket".to_string()) {
        let pair = w::WebSocketPair::new()?;
        let server = pair.server;
        server.accept()?;
        w::wasm_bindgen_futures::spawn_local(websocket::handle_ws_server(env, server));
        w::Response::from_websocket(pair.client)
    } else {
        w::Response::from_html("OK")
    }
}
