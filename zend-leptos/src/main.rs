use leptos::*;
use zend_leptos::{App, AppProps};

fn main() {
    zend_common::set_panic_hook!();
    mount_to_body(|cx| view! { cx,  <App/> });
}
