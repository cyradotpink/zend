use crate::websocket_api;
use serde::Serialize;
use worker as w;

#[derive(Serialize)]
pub struct CheckNonceMessage {
    #[serde(flatten)]
    pub nonce: websocket_api::Nonce,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "message_type")]
pub enum ToPeerMessage {
    CheckNonceIsUsed(CheckNonceMessage),
}

pub fn make_request(message: &ToPeerMessage) -> Result<w::Request, w::Error> {
    w::Request::new_with_init(
        "/",
        w::RequestInit::new()
            .with_method(w::Method::Post)
            .with_body(Some(w::wasm_bindgen::JsValue::from_str(
                serde_json::to_string(message)?.as_str(),
            ))),
    )
}
