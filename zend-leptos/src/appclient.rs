use crate::wsclient::WsApiClient;
use zend_common::{api, util};

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
    fn decode(&self, key: Aes256GcmKey) -> Result<String, &'static str> {
        use aes_gcm::aead::Aead;
        use aes_gcm::KeyInit;

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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EncodedDataCipherPeer {
    ecdh_public_key: EcdhPublicKey,
    hkdf_salt: HkdfSalt,
    aes_iv: Aes256GcmIv,
    aes_text: String,
}
impl EncodedDataCipherPeer {
    fn decode(&self, key: ecdh::EphemeralSecret) -> Result<String, &'static str> {
        use aes_gcm::aead::Aead;
        use aes_gcm::KeyInit;

        let shared = key.diffie_hellman(&self.ecdh_public_key.0);
        let hkdf = shared.extract::<sha2::Sha256>(Some(&self.hkdf_salt.0));
        let mut okm: [u8; 32] = [0; 32];
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
    plain_text: String,
}
impl EncodedDataTextPlain {
    fn decode(self) -> String {
        self.plain_text
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "cipher_type")]
enum CipherInfo {
    Room(EncodedDataCipherRoom),
    Peer(EncodedDataCipherPeer),
    Plain(EncodedDataTextPlain),
}

struct EncodedData {
    room_id: api::RoomId,
    sender_id: api::EcdsaPublicKeyWrapper,
    nonce: api::Nonce,
    cipher_info: CipherInfo,
}
impl EncodedData {
    fn from_message(data: api::SubscriptionData) -> Result<Self, &'static str> {
        #[derive(Debug, Deserialize)]
        struct CipherPart {
            cipher_info: String,
            signature: api::EcdsaSignatureWrapper,
        }
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

struct DecodedData {
    plain_data: (),
    room_id: api::RoomId,
    sender_id: api::EcdsaPublicKeyWrapper,
    nonce: api::Nonce,
}

struct JoinedRoomInfo {
    room_key: aes_gcm::Key<aes_gcm::Aes256Gcm>,
}

pub struct RoomTextMessage {
    pub text: String,
    nonce: api::Nonce,
    pub sender_id: api::EcdsaPublicKeyWrapper,
}

pub struct CurrentRoomInfo {
    room_id: api::RoomId,
    ecdh_secret: ecdh::EphemeralSecret,
    ecdh_public_key: p256::PublicKey,
    pub ecdsa_verifying_key: ecdsa::VerifyingKey,
    ecdsa_signing_key: ecdsa::SigningKey,
    joined_room_info: Option<JoinedRoomInfo>,
    pub messages: Vec<RoomTextMessage>,
}

pub struct AppState {
    pub current_room: Option<CurrentRoomInfo>,
}

pub struct AppClient {
    api_client: WsApiClient,
    app_state: AppState,
}
