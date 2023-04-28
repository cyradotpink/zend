mod peer_api;
mod room_api;
mod websocket;
mod websocket_api_handlers;

use worker::*;

/*
TODO
refactor:
    - end my habit of placing children above their parents.
    - reduce match statement nesting.
*/

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.headers().get("Upgrade")? == Some("websocket".to_string()) {
        let pair = WebSocketPair::new()?;
        let server = pair.server;
        server.accept()?;
        wasm_bindgen_futures::spawn_local(websocket::handle_ws_server(env, server));
        Response::from_websocket(pair.client)
    } else {
        Response::from_html("OK")
    }
}