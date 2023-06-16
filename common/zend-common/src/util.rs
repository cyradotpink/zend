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
pub fn decode_base64_slice(
    value: &str,
    output: &mut [u8],
) -> Result<usize, base64::DecodeSliceError> {
    base64::Engine::decode_slice(&base64::engine::general_purpose::STANDARD, value, output)
}
pub fn decode_base64_slice_exact(
    value: &str,
    length: usize,
    output: &mut [u8],
) -> Result<(), &'static str> {
    decode_base64_slice(value, output)
        .map_err(|_| "Base64 decode error")?
        .eq(&length)
        .then_some(())
        .ok_or("Bad IV length")
}

#[macro_export]
macro_rules! debug_log_pretty {
    ($x:expr) => {
        $crate::log!("{:#?}", $x)
    };
}

#[macro_export]
macro_rules! debug_log {
    ($x:expr) => {
        $crate::log!("{:?}", $x)
    };
}

#[macro_export]
macro_rules! log {
    () => {
        $crate::log!("")
    };
    ($($arg:tt)*) => {{
        let arr = $crate::_use::js_sys::Array::new_with_length(3);
        arr.set(
            0,
            $crate::_use::wasm_bindgen::JsValue::from_str(&format!("%c[{}:{}]", ::std::file!(), ::std::line!())),
        );
        arr.set(1, $crate::_use::wasm_bindgen::JsValue::from_str("font-weight: bold"));
        let s = ::std::fmt::format(format_args!($($arg)*));
        arr.set(2, $crate::_use::wasm_bindgen::JsValue::from_str(&s));
        $crate::_use::web_sys::console::log(&arr);
    }};
}
