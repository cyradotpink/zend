use leptos::*;
use leptos_router::*;
mod appclient;
mod util;
mod wsclient;
use zend_common::{_use::wasm_bindgen::UnwrapThrowExt, api, debug_log_pretty};

#[component]
pub fn App(cx: Scope) -> impl IntoView {
    // let _ws = wsclient::WsApiClient::new("ws://localhost:8787");
    let mut client = appclient::AppClient::new();
    // debug_log_pretty!(client);
    let message = client.make_server_method_call(api::SubscribeToRoomArgs {
        room_id: api::RoomId::from_int(0),
    });
    let json = serde_json::to_string(&message);
    debug_log_pretty!(json);
    let message = client.make_server_method_call(api::BroadcastDataArgs {
        common_args: api::SendDataCommonArgs {
            room_id: api::RoomId::from_int(0),
            write_history: false,
            data: serde_json::from_str("\"\"").unwrap_throw(),
        },
    });
    let json = serde_json::to_string(&message);
    debug_log_pretty!(json);

    view! { cx,
        <Router>
            <Routes>
                <Route path="/" view=|cx| view! { cx, <div></div> }/>
                <Route path="/room/:id" view=|cx| view! { cx, <div></div> }/>
                <Route path="/*any" view=|cx| view! { cx, <Redirect path="/"/> }/>
            </Routes>
        </Router>
    }
}
