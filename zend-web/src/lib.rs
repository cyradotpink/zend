mod wsclient;

use std::{error::Error, format};
use wasm_bindgen::prelude::*;
use zend_common::log;

async fn program_main() -> Result<(), Box<dyn Error>> {
    let _ws = wsclient::WsApiClient::new("ws://localhost:8787");
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
