mod wsclient;

use std::{error::Error, format, time::Duration};
use wasm_bindgen::prelude::*;
use zend_common::log;

async fn program_main() -> Result<(), Box<dyn Error>> {
    let ws = wsclient::WsApiClient::new("ws://localhost:8787");
    let ws_clone = ws.clone();
    wasm_bindgen_futures::spawn_local(async move {
        for _ in 0..5 * 15 {
            gloo_timers::future::sleep(Duration::from_millis(200)).await;
            web_sys::window()
                .unwrap_throw()
                .document()
                .unwrap_throw()
                .body()
                .unwrap_throw()
                .set_inner_html(&format!("<pre>{:#?}</pre>", ws));
        }
        log!("printer task ending")
    });
    let ws = ws_clone;
    let handle = ws.receive_events(wsclient::SubscriptionEventFilter::Any);
    //while let Some(v) = handle.receiver.next().await {
    //    log(&format!("{:?}", v))
    //}
    gloo_timers::future::sleep(Duration::from_secs(5)).await;
    handle.unsubscribe();
    log!("dropped handle");
    ws.end();
    gloo_timers::future::sleep(Duration::from_secs(5)).await;
    log!("main ending");
    Ok(())
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|v| {
        log!("Rust panicked qwq\n{}", v);
    }));
    wasm_bindgen_futures::spawn_local(async {
        let result = program_main().await;
        log!("{:?}", result);
    });
    Ok(())
}
