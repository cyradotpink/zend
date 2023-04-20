use getrandom::getrandom;

/** Simulates Math.random() using getrandom */
pub fn math_random() -> Result<f64, ()> {
    let mut random = [0u8; 4];
    getrandom(&mut random).map_err(|_| ())?;
    let random = u32::from_be_bytes(random);
    Ok(random as f64 / u32::MAX as f64)
}

pub fn encode_base64(value: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value)
}
pub fn decode_base64(value: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
}
