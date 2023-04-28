use serde::Serialize;
use worker as w;
use zend_common::{api, enum_convert::EnumConvert};

#[derive(Serialize)]
pub struct InitialiseMessage {
    pub initial_peer_id: api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct SubscribeMessage {
    pub subscriber_id: api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct UnsubscribeMessage {
    pub subscription_id: u64,
}

#[derive(Serialize)]
pub struct AddPrivilegedPeerMessage {
    pub adder_id: api::EcdsaPublicKeyWrapper,
    pub added_id: api::EcdsaPublicKeyWrapper,
}

#[derive(Serialize)]
pub struct DeleteMessage {
    pub deleter_id: Option<api::EcdsaPublicKeyWrapper>,
}

#[derive(Serialize)]
pub struct BroadcastDataMessage {
    pub data: serde_json::Value,
    pub sender_id: api::EcdsaPublicKeyWrapper,
    pub nonce: api::Nonce,
    pub write_history: bool,
}

#[derive(Serialize)]
pub struct UnicastDataMessage {
    pub data: serde_json::Value,
    pub sender_id: api::EcdsaPublicKeyWrapper,
    pub receiver_id: api::EcdsaPublicKeyWrapper,
    pub nonce: api::Nonce,
    pub write_history: bool,
}

#[derive(Serialize)]
pub struct DeleteDataMessage {
    pub deleter_id: api::EcdsaPublicKeyWrapper,
    pub data_sender_id: api::EcdsaPublicKeyWrapper,
    pub data_nonce: api::Nonce,
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
