mod wsclient;

use std::{error::Error, rc::Rc, time::Duration};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

async fn program_main() -> Result<(), Box<dyn Error>> {
    let ws = wsclient::WsRefCellWrap::new("ws://localhost:8787", Some(Duration::from_secs(15)));
    let ws = Rc::new(ws);
    let move_ref = ws.clone();
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            gloo_timers::future::sleep(Duration::from_secs(10)).await;
            move_ref.send(
                &serde_json::to_string(&zend_common::api::ClientToServerMessage::Ping)
                    .unwrap_throw(),
            );
        }
    });
    while let Some(event) = ws.next_event().await {
        log(&format!("{:?}", event));
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
