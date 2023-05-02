mod peer_api;
mod room_api;
mod websocket;
mod websocket_api_handlers;

use std::cell::Cell;
use worker::*;

thread_local!(static HOOK_SET: Cell<bool> = Cell::new(false));

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    HOOK_SET.with(|is_set| {
        if !is_set.get() {
            zend_common::log!("Set panic hook :3");
            std::panic::set_hook(Box::new(|v: &std::panic::PanicInfo| {
                zend_common::log!("Rust panicked qwq\n{}", v);
            }));
            is_set.set(true);
        }
    });
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
