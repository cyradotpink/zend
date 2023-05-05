use leptos::*;
use leptos_router::*;
mod wsclient;

#[component]
pub fn App(cx: Scope) -> impl IntoView {
    let _ws = wsclient::WsApiClient::new("ws://localhost:8787");

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
