#![allow(dead_code)]

use crate::wsclient::WsApiClient;
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use std::{
    fmt::Debug,
    time::{Duration, SystemTime},
};
use zend_common::{
    _use::wasm_bindgen::UnwrapThrowExt,
    api::{self, EcdsaSignatureWrapper},
    util,
};

use p256::{
    ecdh,
    ecdsa::{self, signature::Verifier},
};
use serde::{Deserialize, Serialize};
use serde_json;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "&str", into = "String")]
struct EcdhPublicKey(pub p256::PublicKey);
impl TryFrom<&str> for EcdhPublicKey {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(Self(
            p256::PublicKey::from_sec1_bytes(
                util::decode_base64(&value)
                    .map_err(|_| "Base64 decode error")?
                    .as_slice(),
            )
            .map_err(|_| "Couldn't decode bytes as p256 key")?,
        ))
    }
}
impl Into<String> for EcdhPublicKey {
    fn into(self) -> String {
        util::encode_base64(&self.0.to_sec1_bytes())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "&str", into = "String")]
struct Aes256GcmKey(pub aes_gcm::Key<aes_gcm::Aes256Gcm>);
impl TryFrom<&str> for Aes256GcmKey {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut output: [u8; 12] = [0; 12];
        util::decode_base64_slice_exact(value, 12, &mut output)?;
        let key: &aes_gcm::Key<aes_gcm::Aes256Gcm> = output.as_slice().into();
        Ok(Self(*key))
    }
}
impl Into<String> for Aes256GcmKey {
    fn into(self) -> String {
        util::encode_base64(&self.0.as_slice())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "&str", into = "String")]
struct Aes256GcmIv(pub [u8; 12]);
impl TryFrom<&str> for Aes256GcmIv {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut output: [u8; 12] = [0; 12];
        util::decode_base64_slice_exact(value, 12, &mut output)?;
        Ok(Self(output))
    }
}
impl Into<String> for Aes256GcmIv {
    fn into(self) -> String {
        util::encode_base64(&self.0)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "&str", into = "String")]
struct HkdfSalt(pub [u8; 32]);
impl TryFrom<&str> for HkdfSalt {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut output: [u8; 32] = [0; 32];
        util::decode_base64_slice_exact(value, 32, &mut output)?;
        Ok(Self(output))
    }
}
impl Into<String> for HkdfSalt {
    fn into(self) -> String {
        util::encode_base64(&self.0)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EncodedDataCipherRoom {
    aes_text: String,
    aes_iv: Aes256GcmIv,
}
impl EncodedDataCipherRoom {
    fn decrypt(&self, key: &Aes256GcmKey) -> Result<String, &'static str> {
        let cipher = aes_gcm::Aes256Gcm::new(&key.0);
        String::from_utf8(
            cipher
                .decrypt(
                    (&self.aes_iv.0).into(),
                    util::decode_base64(&self.aes_text)
                        .map_err(|_| "Failed to decode room-encrypted ciphertext base64")?
                        .as_slice(),
                )
                .map_err(|_| "Failed to decrypt room-encrypted ciphertext")?,
        )
        .map_err(|_| "Failed to utf8-decode room-encrypted ciphertext's plaintext")
    }
    fn encrypt(key: &aes_gcm::Key<aes_gcm::Aes256Gcm>, iv: [u8; 12], plaintext: String) -> Self {
        let cipher = Aes256Gcm::new(key);
        let cipher_text = cipher
            .encrypt(&iv.into(), plaintext.as_bytes())
            .unwrap_throw();
        Self {
            aes_text: util::encode_base64(&cipher_text),
            aes_iv: Aes256GcmIv(iv),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EncodedDataCipherPeer {
    ecdh_public_key: EcdhPublicKey,
    hkdf_salt: HkdfSalt,
    aes_iv: Aes256GcmIv,
    aes_text: String,
}
impl EncodedDataCipherPeer {
    fn decrypt(&self, key: &ecdh::EphemeralSecret) -> Result<String, &'static str> {
        let shared = key.diffie_hellman(&self.ecdh_public_key.0);
        let hkdf = shared.extract::<sha2::Sha256>(Some(&self.hkdf_salt.0));
        let mut okm = [0u8; 32];
        hkdf.expand(&[], &mut okm)
            .map_err(|_| "Failed to use ECDH shared secret as AES key material")?;
        let hkdf_derived_key: &aes_gcm::Key<aes_gcm::Aes256Gcm> = okm.as_slice().into();
        let cipher = aes_gcm::Aes256Gcm::new(&hkdf_derived_key);
        String::from_utf8(
            cipher
                .decrypt(
                    (&self.aes_iv.0).into(),
                    util::decode_base64(&self.aes_text)
                        .map_err(|_| "Failed to decode peer-encrypted ciphertext base64")?
                        .as_slice(),
                )
                .map_err(|_| "Failed to decrypt peer-encrypted ciphertext")?,
        )
        .map_err(|_| "Failed to utf8-decode peer-encrypted ciphertext's plaintext")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EncodedDataTextPlain {
    pub plain_text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "cipher_type")]
enum CipherInfo {
    Room(EncodedDataCipherRoom),
    Peer(EncodedDataCipherPeer),
    Plain(EncodedDataTextPlain),
}

#[derive(Debug, Deserialize, Serialize)]
struct CipherPart {
    cipher_info: String,
    signature: api::EcdsaSignatureWrapper,
}
impl CipherPart {
    fn with_room_key(
        room_key: &aes_gcm::Key<aes_gcm::Aes256Gcm>,
        signing_key: &ecdsa::SigningKey,
        iv: [u8; 12],
        call: &RoomMethodCall,
    ) -> Self {
        use p256::ecdsa::signature::Signer;

        let call_json = serde_json::to_string(call).unwrap_throw();
        let encoded = EncodedDataCipherRoom::encrypt(room_key, iv, call_json);
        let cipher_info = CipherInfo::Room(encoded);
        let cipher_info_json = serde_json::to_string(&cipher_info).unwrap_throw();

        Self {
            signature: EcdsaSignatureWrapper(signing_key.sign(cipher_info_json.as_bytes())),
            cipher_info: cipher_info_json,
        }
    }
}

struct EncodedData {
    room_id: api::RoomId,
    sender_id: api::EcdsaPublicKeyWrapper,
    nonce: api::Nonce,
    cipher_info: CipherInfo,
}
impl EncodedData {
    fn from_message(data: api::SubscriptionData) -> Result<Self, &'static str> {
        let cipher_part: CipherPart =
            serde_json::from_value(data.data).map_err(|_| "Error parsing CipherPart")?;
        let cipher_info: CipherInfo = serde_json::from_str(&cipher_part.cipher_info)
            .map_err(|_| "Error parsing CipherInfo")?;
        let normalized = format!(
            "{}&{}&{}&{}",
            data.sender_id.to_string(),
            data.room_id.to_string(),
            data.nonce.to_string(),
            cipher_part.cipher_info
        );
        data.sender_id
            .0
            .verify(&normalized.as_bytes(), &cipher_part.signature.0)
            .map_err(|_| "ECDSA authentication failed")?;
        Ok(Self {
            room_id: data.room_id,
            sender_id: data.sender_id,
            nonce: data.nonce,
            cipher_info,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum RoomMethodCall {
    AcceptJoin {
        room_key: Aes256GcmKey,
    },
    InitJoin {
        joining_id: EcdhPublicKey,
    },
    SendMessage {
        message: String,
    },
    DeleteMessage {
        target_nonce: api::Nonce,
        sender_id: api::EcdsaPublicKeyWrapper,
    },
    ConfirmJoin {
        joined_id: api::EcdsaPublicKeyWrapper,
    },
    PreventJoin {
        denied_id: api::EcdsaPublicKeyWrapper,
    },
}

struct DecodedData {
    method_call: RoomMethodCall,
    room_id: api::RoomId,
    sender_id: api::EcdsaPublicKeyWrapper,
    nonce: api::Nonce,
}
impl DecodedData {
    fn from_encoded_data(
        data: EncodedData,
        aes_key: &Aes256GcmKey,
        ecdh_secret: &ecdh::EphemeralSecret,
    ) -> Result<Self, &'static str> {
        let info_json = match data.cipher_info {
            CipherInfo::Room(info) => info.decrypt(aes_key)?,
            CipherInfo::Peer(info) => info.decrypt(ecdh_secret)?,
            CipherInfo::Plain(info) => info.plain_text,
        };
        let call: RoomMethodCall = serde_json::from_str(&info_json)
            .map_err(|_| "Failed to deserialise method call JSON")?;
        Ok(Self {
            method_call: call,
            room_id: data.room_id,
            sender_id: data.sender_id,
            nonce: data.nonce,
        })
    }
}

struct JoinedRoomInfo {
    room_key: aes_gcm::Key<aes_gcm::Aes256Gcm>,
    room_id: api::RoomId,
}

#[derive(Debug)]
pub struct RoomTextMessage {
    text: String,
    nonce: api::Nonce,
    sender_id: api::EcdsaPublicKeyWrapper,
}

// Valid state transitions are:
// NoRoom -> CreatingRoom
// NoRoom -> JoiningRoom
// CreatingRoom -> InRoom
// Joiningroom -> Inroom
// InRoom -> NoRoom (By AppState reinit)
#[derive(Debug)]
pub enum CurrentAppState {
    NoRoom,
    CreatingRoom,
    JoiningRoom {
        room_id: api::RoomId,
    },
    InRoom {
        room_id: api::RoomId,
        room_key: aes_gcm::Key<aes_gcm::Aes256Gcm>,
    },
}

pub struct RoomState {
    current_state: CurrentAppState,
    ecdh_secret: ecdh::EphemeralSecret,
    ecdh_public_key: p256::PublicKey,
    ecdsa_verifying_key: ecdsa::VerifyingKey,
    ecdsa_signing_key: ecdsa::SigningKey,
    messages: Vec<RoomTextMessage>,
    next_nonce: api::Nonce,
    last_time: u64,
}
impl Debug for RoomState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("current_state", &self.current_state)
            .field("messages", &self.messages)
            .field("next_nonce", &self.next_nonce)
            .field("last_time", &self.last_time)
            .finish()
    }
}
fn get_sys_time() -> u64 {
    (js_sys::Date::now() / 1000f64) as u64
}
impl RoomState {
    pub fn init() -> Self {
        let ecdh_secret = ecdh::EphemeralSecret::random(&mut rand_core::OsRng);
        let ecdh_public_key = ecdh_secret.public_key();
        let ecdsa_signing_key = ecdsa::SigningKey::random(&mut rand_core::OsRng);
        let ecdsa_verifying_key = ecdsa::VerifyingKey::from(&ecdsa_signing_key);
        let time = get_sys_time();
        Self {
            current_state: CurrentAppState::NoRoom,
            ecdh_secret,
            ecdh_public_key,
            ecdsa_verifying_key,
            ecdsa_signing_key,
            messages: Vec::new(),
            next_nonce: api::Nonce::new(time),
            last_time: time,
        }
    }
    fn reinit(&mut self) {
        *self = Self::init();
    }
    fn get_time(&mut self) -> u64 {
        let now = std::cmp::max(self.last_time, get_sys_time());
        self.last_time = now;
        now
    }
    fn next_nonce(&mut self) -> api::Nonce {
        let time = self.get_time();
        let nonce = self.next_nonce;
        self.next_nonce.increment(time);
        nonce
    }
}

#[derive(Debug)]
pub struct AppClient {
    api_client: WsApiClient,
    room_state: RoomState,
    next_call_id: u64,
}
impl AppClient {
    pub fn new() -> Self {
        Self {
            api_client: WsApiClient::new("https://garbage.notaws"),
            room_state: RoomState::init(),
            next_call_id: 0,
        }
    }
    pub fn make_server_method_call<T: Into<api::MethodCallArgsVariants>>(
        &mut self,
        args: T,
    ) -> api::ClientToServerMessage {
        // let args: api::MethodCallArgsVariants = args.into();
        let call = api::MethodCallContent::new(
            api::EcdsaPublicKeyWrapper(self.room_state.ecdsa_verifying_key),
            self.room_state.next_nonce(),
            args.into(),
        );
        let call = call
            .sign(self.next_call_id, &self.room_state.ecdsa_signing_key)
            .unwrap_throw();
        self.next_call_id += 1;
        call.into()
    }
}
