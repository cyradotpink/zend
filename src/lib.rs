mod peer_api;
mod room_api;
mod websocket;
mod websocket_api;

use futures::StreamExt;
use std::{borrow::Cow, rc::Rc};
use w::console_log;
use worker as w;

fn make_internal_error() -> w::Error {
    w::Error::Internal(w::wasm_bindgen::JsValue::from_str("internal"))
}

fn get_query_value<'a>(url: &'a w::Url, key: &str) -> Result<Cow<'a, str>, w::Error> {
    let (_, v) = url
        .query_pairs()
        .find(|(k, _)| k == key)
        .ok_or(make_internal_error())?;
    Ok(v)
}

#[w::event(fetch)]
pub async fn fetch(req: w::Request, env: w::Env, _ctx: w::Context) -> w::Result<w::Response> {
    if req.headers().get("Upgrade")? == Some("websocket".to_string()) {
        let pair = w::WebSocketPair::new()?;
        let server = pair.server;
        server.accept()?;
        // let env = Rc::new(env);
        w::wasm_bindgen_futures::spawn_local(websocket::handle_ws_server(env, server));
        w::Response::from_websocket(pair.client)
    } else {
        let mut response = env
            .durable_object("ROOM")?
            .id_from_name("AAAAAA")?
            .get_stub()?
            .fetch_with_request(req)
            .await?;
        console_log!("{}", response.text().await?);
        w::Response::from_html("OK")
    }
}
