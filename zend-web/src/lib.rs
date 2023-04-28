mod wsclient;

use futures::StreamExt;
use std::error::Error;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

async fn program_main() -> Result<(), Box<dyn Error>> {
    let ws = wsclient::WsApiClient::new("ws://localhost:8787");
    let mut receiver = ws.receive_events(wsclient::SubscriptionEventFilter::Any);
    while let Some(ev) = receiver.next().await {
        log(&format!("ApiClientEvent: {:?}", ev));
    }
    Ok(())
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    log("Exec start");
    wasm_bindgen_futures::spawn_local(async {
        let result = program_main().await;
        log(&format!("{:?}", result));
    });
    Ok(())
}
