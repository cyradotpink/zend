use base64;
use enum_convert::EnumConvert;
use p256::{
    ecdsa,
    ecdsa::{signature::Verifier, Signature},
};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fmt::Display;

fn encode_base64(value: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
}
fn decode_base64(value: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
}

#[derive(Copy, Clone, Deserialize, Serialize, Debug)]
#[serde(try_from = "String")]
#[serde(into = "String")]
pub struct Nonce {
    pub id: u64,
    pub timestamp: u64,
}
impl TryFrom<String> for Nonce {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let mut segments = value.split('_');
        let id = u64::from_str_radix(segments.next().ok_or("No ID segment.")?, 10)
            .map_err(|_| "Invalid ID segment.")?;
        let timestamp = u64::from_str_radix(segments.next().ok_or("No timestamp segment.")?, 10)
            .map_err(|_| "Invalid timestamp segment.")?;
        if segments.next().is_some() {
            return Err("Too many segments".to_string());
        }
        Ok(Self { id, timestamp })
    }
}
impl Display for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}_{}", self.id, self.timestamp))
    }
}
impl Into<String> for Nonce {
    fn into(self) -> String {
        self.to_string()
    }
}

#[derive(Deserialize, Clone, Serialize, Debug)]
#[serde(try_from = "String", into = "String")]
pub struct EcdsaPublicKeyWrapper(ecdsa::VerifyingKey);
#[derive(Debug, EnumConvert)]
#[enum_convert(from)]
pub enum VerifyingKeyFromBase64Error {
    BytesFromBase64Error(base64::DecodeError),
    KeyFromBytesError(p256::ecdsa::Error),
}
impl Display for VerifyingKeyFromBase64Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}
impl TryFrom<String> for EcdsaPublicKeyWrapper {
    type Error = VerifyingKeyFromBase64Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = decode_base64(&value)?;
        Ok(Self(ecdsa::VerifyingKey::from_sec1_bytes(&bytes)?))
    }
}
impl Into<String> for EcdsaPublicKeyWrapper {
    fn into(self) -> String {
        self.to_string()
    }
}
impl Display for EcdsaPublicKeyWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&encode_base64(&self.0.to_sec1_bytes()))
    }
}

#[derive(Deserialize, Clone, Serialize, Debug)]
#[serde(try_from = "String")]
#[serde(into = "String")]
struct EcdsaSignatureWrapper(Signature);
#[derive(Debug, EnumConvert)]
#[enum_convert(from)]
enum SignatureFromBase64Error {
    BytesFromBase64Error(base64::DecodeError),
    SignatureFromBytesError(ecdsa::signature::Error),
}
impl Display for SignatureFromBase64Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}
impl TryFrom<String> for EcdsaSignatureWrapper {
    type Error = SignatureFromBase64Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = decode_base64(&value)?;
        Ok(Self(Signature::from_slice(&bytes.as_slice())?))
    }
}
impl Into<String> for EcdsaSignatureWrapper {
    fn into(self) -> String {
        encode_base64(&self.0.to_bytes())
    }
}
impl Display for EcdsaSignatureWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&<Self as Into<String>>::into(self.clone()))
    }
}

#[derive(Deserialize, Clone, Serialize, Debug)]
#[serde(try_from = "String")]
#[serde(into = "String")]
pub struct RoomId(pub u64);
impl RoomId {
    pub fn random(gen: fn() -> f64) -> Self {
        Self((gen() * 26u64.pow(6) as f64) as u64)
    }
}
impl TryFrom<String> for RoomId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let mut out_int = 0;
        let mut exponent = 5i8;
        for mut char in value.chars() {
            if exponent < 0 {
                return Err("ID too long".into());
            }
            char.make_ascii_uppercase();
            if !char.is_ascii_uppercase() {
                return Err("ID contains invalid characters".into());
            }
            let value = (char as u64) - 65;
            out_int = out_int + 26u64.pow(exponent as u32) * value;
            exponent = exponent - 1;
        }
        if exponent > -1 {
            return Err("ID too short".into());
        }
        Ok(Self(out_int))
    }
}
impl Into<String> for RoomId {
    fn into(self) -> String {
        let mut out = String::with_capacity(6);
        // Some potential for subtle bugs as values that are too large to be RoomIds
        // are silently moduloed into the required range, instead of causing an error.
        // Implemented this way because serde does not offer a try_into macro.
        let mut input = self.0 % 26u64.pow(6);
        let mut i = 0_usize;
        while i < 6 {
            if input > 0 {
                out.push((input % 26 + 65) as u8 as char);
                input = input / 26;
            } else {
                out.push('A');
            }
            i = i + 1;
        }
        out.chars().rev().collect()
    }
}
impl Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&<Self as Into<String>>::into(self.clone()))
    }
}

#[derive(Deserialize, Debug)]
pub struct MethodCallCommonArgs {
    pub ecdsa_public_key: EcdsaPublicKeyWrapper,
    pub nonce: Nonce,
}

#[derive(Deserialize, Debug)]
pub struct SubscribeToRoomArgs {
    pub room_id: RoomId,
}

#[derive(Deserialize, Debug)]
pub struct AddPrivilegedPeerArgs {
    pub room_id: RoomId,
    pub allow_ecdsa_public_key: EcdsaPublicKeyWrapper,
}

#[derive(Deserialize, Debug)]
pub struct GetRoomDataHistoryArgs {
    pub room_id: RoomId,
    pub from_timestamp: u64,
}

#[derive(Deserialize, Debug)]
pub struct DeleteDataArgs {
    pub room_id: RoomId,
    pub data_sender_key: EcdsaPublicKeyWrapper,
    pub data_nonce: Nonce,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SendDataCommonArgs {
    room_id: RoomId,
    write_history: bool,
    timestamp: u64,
    // #[serde(flatten)]
    // data: SendDataDataVariants,
    data: serde_json::Value,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct BroadcastDataArgs {
    #[serde(flatten)]
    pub common_args: SendDataCommonArgs,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct UnicastDataArgs {
    pub receiver_ecdsa_public_key: EcdsaPublicKeyWrapper,
    #[serde(flatten)]
    pub common_args: SendDataCommonArgs,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "method_name", content = "method_arguments")]
#[serde(rename_all = "snake_case")]
pub enum MethodCallArgsVariants {
    CreateRoom,
    SubscribeToRoom(SubscribeToRoomArgs),
    AddPrivilegedPeer(AddPrivilegedPeerArgs),
    GetRoomDataHistory(GetRoomDataHistoryArgs),
    DeleteData(DeleteDataArgs),
    BroadcastData(BroadcastDataArgs),
    UnicastData(UnicastDataArgs),
}

#[derive(Deserialize, Debug)]
#[serde(try_from = "serde_json::Value")]
pub struct MethodCallContent {
    pub common_arguments: MethodCallCommonArgs,
    pub variant_arguments: MethodCallArgsVariants,
}

impl TryFrom<serde_json::Value> for MethodCallContent {
    type Error = serde_json::Error;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        #[derive(Deserialize, Debug)]
        struct CommonArgsHelper {
            method_arguments: MethodCallCommonArgs,
        }
        // Cloning here is probably not ideal but whatever
        let common_arguments =
            serde_json::from_value::<CommonArgsHelper>(value.clone())?.method_arguments;
        let variant_arguments: MethodCallArgsVariants = serde_json::from_value(value)?;
        Ok(Self {
            common_arguments,
            variant_arguments,
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(try_from = "String")]
pub struct MethodCall {
    json: String,
    pub call: MethodCallContent,
}

impl TryFrom<String> for MethodCall {
    type Error = serde_json::Error;
    fn try_from(value_json: String) -> Result<Self, Self::Error> {
        let content = serde_json::from_str(&value_json)?;
        Ok(Self {
            json: value_json,
            call: content,
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct SignedMethodCallPartial {
    pub call_id: u64,
    #[serde(flatten)]
    extra: serde_json::Value,
}

#[derive(Deserialize, Debug)]
pub struct SignedMethodCall {
    pub call_id: u64,
    pub signed_call: MethodCall,
    signature: EcdsaSignatureWrapper,
}
impl SignedMethodCall {
    pub fn validate_timestamp(&self, now: u64) -> bool {
        let timestamp = self.signed_call.call.common_arguments.nonce.timestamp;
        timestamp < now + 10 && timestamp > now - 5 * 60
    }
    pub fn validate_signature(&self) -> Result<(), p256::ecdsa::Error> {
        self.signed_call
            .call
            .common_arguments
            .ecdsa_public_key
            .0
            .verify(self.signed_call.json.as_bytes(), &self.signature.0)
    }
}

#[derive(Deserialize, Debug, EnumConvert)]
#[enum_convert(from, into)]
#[serde(try_from = "SignedMethodCallPartial")]
pub enum SignedMethodCallOrPartial {
    Partial(u64),
    Full(SignedMethodCall),
}
impl From<SignedMethodCallPartial> for SignedMethodCallOrPartial {
    fn from(value: SignedMethodCallPartial) -> Self {
        fn fallible(mut value: SignedMethodCallPartial) -> Result<SignedMethodCall, ()> {
            value.extra.as_object_mut().ok_or(())?.insert(
                "call_id".to_string(),
                serde_json::to_value(value.call_id).map_err(|_| ())?,
            );
            serde_json::from_value(value.extra).map_err(|_| ())
        }
        let call_id = value.call_id;
        match fallible(value) {
            Ok(v) => Self::Full(v),
            Err(_) => Self::Partial(call_id),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(
    rename_all = "snake_case",
    tag = "message_type",
    content = "message_content"
)]
pub enum ClientToServerMessage {
    Ping,
    SignedMethodCall(SignedMethodCallOrPartial),
}

#[derive(Serialize, Debug)]
pub struct CreateRoomSuccess {
    pub room_id: RoomId,
}

#[derive(Serialize, Debug, EnumConvert)]
#[enum_convert(from)]
#[serde(untagged)]
pub enum MethodCallSuccess {
    // When deserialising, serde should attempt to deserialise to this variant
    // first and immediately succeed, leaving the client to manually deserialise
    // into an actual type.
    Value(serde_json::Value),
    CreateRoom(CreateRoomSuccess),
    Ack,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ErrorId {
    InternalError,
    InvalidSignature,
    ParseError,
}
impl ErrorId {
    pub fn with_message(self, message: String) -> MethodCallError {
        MethodCallError {
            error_id: self,
            message: Some(message),
        }
    }
    pub fn with_default_message(self) -> MethodCallError {
        #[allow(unreachable_patterns)]
        let message = match self {
            ErrorId::InternalError => "An unexpected internal error occured.",
            ErrorId::InvalidSignature => "The request was not signed correctly.",
            ErrorId::ParseError => "The request could not be parsed.",
            _ => "",
        };
        if message.is_empty() {
            self.with_message(message.to_string())
        } else {
            MethodCallError {
                error_id: self,
                message: None,
            }
        }
    }
}

#[derive(Serialize, Debug)]
pub struct MethodCallError {
    error_id: ErrorId,
    message: Option<String>,
}
impl From<ErrorId> for MethodCallError {
    fn from(error_id: ErrorId) -> Self {
        error_id.with_default_message()
    }
}

#[derive(Serialize, Debug, EnumConvert)]
#[serde(
    rename_all = "snake_case",
    tag = "return_type",
    content = "return_data"
)]
#[enum_convert(from)]
pub enum MethodCallReturnVariants {
    Success(MethodCallSuccess),
    Error(MethodCallError),
}

#[derive(Serialize, Debug)]
pub struct MethodCallReturn {
    pub call_id: u64,
    #[serde(flatten)]
    pub return_data: MethodCallReturnVariants,
}

#[derive(Serialize, Debug, EnumConvert)]
#[enum_convert(from)]
#[serde(
    rename_all = "snake_case",
    tag = "message_type",
    content = "message_content"
)]
pub enum ServerToClientMessage {
    Pong,
    MethodCallReturn(MethodCallReturn),
    Info(String),
}
impl ServerToClientMessage {
    pub fn pong() -> Self {
        Self::Pong
    }
    pub fn call_error(call_id: u64, error_id: ErrorId, message: Option<String>) -> Self {
        MethodCallReturn {
            call_id,
            return_data: MethodCallError { error_id, message }.into(),
        }
        .into()
    }
    pub fn from_error(call_id: u64, error: MethodCallError) -> Self {
        MethodCallReturn {
            call_id,
            return_data: error.into(),
        }
        .into()
    }
    pub fn from_success(call_id: u64, data: MethodCallSuccess) -> Self {
        Self::MethodCallReturn(MethodCallReturn {
            call_id,
            return_data: data.into(),
        })
    }
    pub fn info(text: &str) -> Self {
        Self::Info(text.to_string())
    }
}
