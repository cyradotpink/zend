mod wsclient;

// use futures::StreamExt;
use std::{error::Error, time::Duration};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

fn log_str(s: &str) {
    let arr = js_sys::Array::new();
    arr.push(&JsValue::from_str(s));
    web_sys::console::log(&arr);
}

async fn program_main() -> Result<(), Box<dyn Error>> {
    let ws = wsclient::WsApiClient::new("ws://localhost:8787");

    log_str("sdf");
    let ws_clone = ws.clone();
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            gloo_timers::future::sleep(Duration::from_millis(200)).await;
            web_sys::window()
                .unwrap_throw()
                .document()
                .unwrap_throw()
                .body()
                .unwrap_throw()
                .set_inner_html(&format!("<pre>{:#?}</pre>", ws));
        }
    });
    let ws = ws_clone;
    let handle = ws.receive_events(wsclient::SubscriptionEventFilter::Any);
    //while let Some(v) = handle.receiver.next().await {
    //    log(&format!("{:?}", v))
    //}
    gloo_timers::future::sleep(Duration::from_secs(10)).await;
    handle.unsubscribe();
    log("1");
    gloo_timers::future::sleep(Duration::from_secs(60)).await;
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
