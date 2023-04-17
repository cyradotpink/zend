use enum_convert::EnumConvert;
use serde::Serialize;
use serde_json;
use worker as w;

use crate::websocket_api;

#[derive(Serialize)]
// #[enum_from(enum_path = "ToRoomMessage", enum_variant = "Initialise")]
pub struct InitialiseMessage {
    pub initial_peer_id: websocket_api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct SubscribeMessage {
    pub subscriber_id: websocket_api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct AddPrivilegedPeerMessage {
    pub adder_id: websocket_api::EcdsaPublicKeyWrapper,
    pub added_id: websocket_api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct DeleteMessage {
    pub deleter_id: Option<websocket_api::EcdsaPublicKeyWrapper>,
}

#[derive(Serialize)]
pub struct BroadcastDataMessage {
    pub data: serde_json::Value,
    pub sender_id: websocket_api::EcdsaPublicKeyWrapper,
    pub nonce: websocket_api::Nonce,
    pub write_history: bool,
}

#[derive(Serialize)]
pub struct UnicastDataMessage {
    pub data: serde_json::Value,
    pub sender_id: websocket_api::EcdsaPublicKeyWrapper,
    pub receiver_id: websocket_api::EcdsaPublicKeyWrapper,
    pub nonce: websocket_api::Nonce,
    pub write_history: bool,
}

#[derive(Serialize)]
pub struct DeleteDataMessage {
    pub deleter_id: websocket_api::EcdsaPublicKeyWrapper,
    pub data_sender_id: websocket_api::EcdsaPublicKeyWrapper,
    pub data_nonce: websocket_api::Nonce,
}

#[derive(Serialize, EnumConvert)]
#[enum_convert(from, into)]
#[serde(rename_all = "snake_case", tag = "message_type")]
pub enum ToRoomMessage {
    Initialise(InitialiseMessage),
    // CheckExists,
    Subscribe(SubscribeMessage),
    AddPrivilegedPeer(AddPrivilegedPeerMessage),
    Delete(DeleteMessage),
    BroadcastData(BroadcastDataMessage),
    UnicastData(UnicastDataMessage),
    DeleteData(DeleteDataMessage),
}

pub fn make_request<T: Into<ToRoomMessage>>(message: T) -> Result<w::Request, w::Error> {
    let message: ToRoomMessage = message.into();
    w::Request::new_with_init(
        "/",
        w::RequestInit::new()
            .with_method(w::Method::Post)
            .with_body(Some(w::wasm_bindgen::JsValue::from_str(
                serde_json::to_string(&message)?.as_str(),
            ))),
    )
}

pub trait IntoRequest {
    fn into_request(self) -> Result<w::Request, w::Error>;
}
impl<T: Into<ToRoomMessage>> IntoRequest for T {
    fn into_request(self) -> Result<w::Request, w::Error> {
        make_request(self)
    }
}

impl TryInto<w::Request> for ToRoomMessage {
    type Error = w::Error;
    fn try_into(self) -> Result<w::Request, Self::Error> {
        self.into_request()
    }
}
