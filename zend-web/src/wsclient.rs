use std::{
    cell::{Ref, RefCell},
    rc::Rc,
    time::Duration,
};

use futures::{channel::mpsc, future, stream::StreamExt};
use pharos::{Events, Observable, ObserveConfig};
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
use ws_stream_wasm::{WsEvent, WsMessage, WsMeta, WsStream};
use zend_common::api;

macro_rules! let_is {
    ($p:pat = $i:ident) => {
        if let $p = $i {
            true
        } else {
            false
        }
    };
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[derive(Debug)]
pub enum WrappedSocketEvent {
    Connected,
    /// Seconds until next reconnection attempt
    Reconnecting(u64),
    TextMessage(String),
    BinaryMessage(Vec<u8>),
    Ended(&'static str),
}
struct WebSocketWrap {
    finished: bool,
    url: String, // Could maybe be a &str but not really worth it I think
    ws: Option<(WsStream, Events<WsEvent>)>,
    retry_after: u64,
    close_timeout: Duration,
    end_on_clean_close: bool,
}
impl WebSocketWrap {
    fn new(url: &str, end_on_clean_close: bool, close_timeout: Option<Duration>) -> Self {
        Self {
            finished: false,
            url: url.into(),
            ws: None,
            retry_after: 0,
            close_timeout: close_timeout.unwrap_or(Duration::MAX),
            end_on_clean_close,
        }
    }

    async fn connect(&mut self) -> Result<(WsStream, Events<WsEvent>), &'static str> {
        let connect_future = Box::pin(WsMeta::connect(&self.url, None));
        let timeout_future = gloo_timers::future::sleep(Duration::from_secs(5));
        let select = future::select(connect_future, timeout_future).await;
        let (mut ws, wsio) = match select {
            future::Either::Left((value, _)) => value.map_err(|_| "WsErr")?,
            future::Either::Right(_) => return Err("Timeout"),
        };
        let events = ws.observe(ObserveConfig::default()).await.unwrap_throw();
        Ok((wsio, events))
    }
    // TODO that is too much nesting. My eyes hurt. stop
    async fn next_event(&mut self) -> Option<WrappedSocketEvent> {
        if self.finished {
            return None;
        }
        if let Some((wsio, events)) = &mut self.ws {
            let timeout_future = gloo_timers::future::sleep(self.close_timeout);
            let next_result = match future::select(wsio.next(), timeout_future).await {
                future::Either::Left((v, _)) => v,
                future::Either::Right(_) => {
                    if let Some((wsio, _)) = self.ws.take() {
                        wsio.wrapped().close().expect_throw(
                            "Something went wrong when closing a websocket connection",
                        );
                    }
                    return Some(WrappedSocketEvent::Reconnecting(self.retry_after));
                }
            };
            if let Some(msg) = next_result {
                return Some(match msg {
                    WsMessage::Text(msg) => WrappedSocketEvent::TextMessage(msg),
                    WsMessage::Binary(msg) => WrappedSocketEvent::BinaryMessage(msg),
                });
            };
            if self.end_on_clean_close {
                let close_event = loop {
                    match events.next().await {
                        Some(WsEvent::Closed(ev)) => break ev,
                        Some(_) => continue,
                        None => {
                            self.finished = true;
                            return Some(WrappedSocketEvent::Ended("Unreachable code reached"));
                        }
                    }
                };
                if close_event.was_clean {
                    self.finished = true;
                    return Some(WrappedSocketEvent::Ended("Clean"));
                }
            }
            self.ws.take();
            return Some(WrappedSocketEvent::Reconnecting(self.retry_after));
        }
        if self.retry_after > 0 {
            gloo_timers::future::sleep(Duration::from_secs(self.retry_after)).await;
            // Exponential backoff maxing out at 60 seconds
            self.retry_after = if self.retry_after * 2 > 60 {
                60
            } else {
                self.retry_after * 2
            };
        } else {
            self.retry_after = 5;
        }
        Some(match self.connect().await {
            Ok(new) => {
                self.retry_after = 0;
                let _ = self.ws.insert(new);
                WrappedSocketEvent::Connected
            }
            Err(_err) => WrappedSocketEvent::Reconnecting(self.retry_after),
        })
    }
}

pub struct WsRefCellWrap {
    ws_wrap: RefCell<WebSocketWrap>,
    ws_copy: RefCell<Option<WebSocket>>,
}
impl WsRefCellWrap {
    pub fn new(url: &str, close_timeout: Option<Duration>) -> Self {
        Self {
            ws_wrap: RefCell::new(WebSocketWrap::new(url, false, close_timeout)),
            ws_copy: RefCell::new(None),
        }
    }
    pub fn send(&self, s: &str) {
        let ws = self.ws_copy.borrow();
        if let Some(ref ws) = *ws {
            let _ = ws.send_with_str(s);
        }
    }
    pub async fn next_event(&self) -> Option<WrappedSocketEvent> {
        let mut wrap = self
            .ws_wrap
            .try_borrow_mut()
            .expect_throw("You ran next_event() twice at the same time. Don't do that :(");
        let event = wrap.next_event().await?;
        match event {
            WrappedSocketEvent::Connected => {
                let mut ws = self.ws_copy.borrow_mut();
                if let Some(new) = &wrap.ws {
                    let _ = ws.insert(new.0.wrapped().clone());
                }
            }
            WrappedSocketEvent::Reconnecting(_) => {
                let mut ws = self.ws_copy.borrow_mut();
                ws.take();
            }
            _ => {}
        }
        Some(event)
    }
}

enum WebSocketState {
    Connected,
    Reconnecting,
    Ended,
}
#[derive(Debug, Clone)]
pub enum ApiClientEvent {
    Connected,
    Reconnecting(u64),
    ApiMessage(zend_common::api::ServerToClientMessage),
    Ended,
}
pub enum SubscriptionEventFilter {
    Any,
    Connected,
    Reconnecting,
    ApiMethodCallReturn(Option<u64>), // Optionally specify call ID
    ApiSubscriptionData(Option<u64>), // Optionally specify subscription ID
    ApiPong,
    ApiInfo,
    Ended,
}
impl Into<Vec<Self>> for SubscriptionEventFilter {
    fn into(self) -> Vec<Self> {
        vec![self]
    }
}

enum EventSubscriptionType {
    Once,
    Persistent,
}
struct EventSubscription {
    event_filters: Vec<SubscriptionEventFilter>,
    sender: mpsc::Sender<ApiClientEvent>,
    subscriber_type: EventSubscriptionType,
}
pub struct WsApiClient {
    // I'm very annoyed by all this reference counting and interior mutability,
    // but it is necessary because these values are used in background tasks-
    // Someone better at Rust would probably find a nicer solution.
    ws: Rc<WsRefCellWrap>,
    subscribers: Rc<RefCell<Vec<EventSubscription>>>,
    ws_state: Rc<RefCell<WebSocketState>>,
}
#[allow(unused)]
impl WsApiClient {
    pub fn new(url: &str) -> Self {
        let subscribers = Rc::new(RefCell::new(Vec::<EventSubscription>::new()));
        let ws = Rc::new(WsRefCellWrap::new(url, Some(Duration::from_secs(30))));
        let ws_state = Rc::new(RefCell::new(WebSocketState::Reconnecting));

        let ws_ref = ws.clone();
        let ws_state_ref = ws_state.clone();
        let subscriptions_ref = subscribers.clone();
        // TODO oneshot(?) channel that ends this task when WsApiClient is dropped
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(event) = ws_ref.next_event().await {
                handle_event(event, &subscriptions_ref, &ws_state_ref)
            }
        });
        let ws_ref = ws.clone();
        // TODO Implement logic to only send when ws state is connected
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::sleep(Duration::from_secs(10)).await;
                ws_ref.send(
                    &serde_json::to_string(&zend_common::api::ClientToServerMessage::Ping)
                        .unwrap_throw(),
                );
            }
        });
        Self {
            ws,
            subscribers,
            ws_state,
        }
    }
    pub fn send_message(&self, message: &api::ClientToServerMessage) {
        let message = match serde_json::to_string(message) {
            Ok(v) => v,
            Err(_) => return,
        };
        self.ws.send(&message);
    }
    fn register_event_subscription(
        &self,
        subscriber_type: EventSubscriptionType,
        event_filters: Vec<SubscriptionEventFilter>,
    ) -> mpsc::Receiver<ApiClientEvent> {
        let (sender, receiver) = mpsc::channel::<ApiClientEvent>(256);
        self.subscribers.borrow_mut().push(EventSubscription {
            event_filters,
            sender,
            subscriber_type,
        });
        receiver
    }
    pub async fn await_one_event<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
    ) -> Result<ApiClientEvent, ()> {
        let mut receiver =
            self.register_event_subscription(EventSubscriptionType::Once, filters.into());
        receiver.next().await.ok_or(())
    }
    pub fn receive_events<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
    ) -> mpsc::Receiver<ApiClientEvent> {
        self.register_event_subscription(EventSubscriptionType::Persistent, filters.into())
    }
}

fn handle_event(
    event: WrappedSocketEvent,
    event_subscriptions: &Rc<RefCell<Vec<EventSubscription>>>,
    ws_state: &Rc<RefCell<WebSocketState>>,
) {
    let event = match event {
        WrappedSocketEvent::Connected => {
            *ws_state.borrow_mut() = WebSocketState::Connected;
            ApiClientEvent::Connected
        }
        WrappedSocketEvent::Reconnecting(v) => {
            *ws_state.borrow_mut() = WebSocketState::Reconnecting;
            ApiClientEvent::Reconnecting(v)
        }
        WrappedSocketEvent::Ended(_) => {
            *ws_state.borrow_mut() = WebSocketState::Ended;
            ApiClientEvent::Ended
        }

        WrappedSocketEvent::TextMessage(msg) => {
            ApiClientEvent::ApiMessage(match serde_json::from_str(&msg) {
                Ok(v) => v,
                Err(_) => return,
            })
        }
        WrappedSocketEvent::BinaryMessage(_) => return,
    };
    // Ref only held until end of loop iteration, before which no .await occurs
    let mut subscribers = event_subscriptions.borrow_mut();
    let mut i = 0;
    loop {
        if i >= subscribers.len() {
            break;
        }
        let subscriber = subscribers
            .get_mut(i)
            .expect_throw("Subscribers list bounds check failed during get");
        let filters = &subscriber.event_filters;

        if !event_is_matched_by_any_filter(&event, filters) {
            i = i + 1;
            continue;
        }
        if let Err(err) = subscriber.sender.try_send(event.clone()) {
            if err.is_disconnected() {
                subscribers.swap_remove(i);
                // Do not increment index here because swap_remove just moved a subscriber to current index
                continue;
            }
        }
        if let EventSubscriptionType::Once = subscriber.subscriber_type {
            subscriber.sender.close_channel();
            subscribers.swap_remove(i);
            // Do not increment index here because swap_remove just moved a subscriber to current index
            continue;
        }
        i = i + 1;
    }
}
fn event_is_matched_by_any_filter(
    event: &ApiClientEvent,
    filters: &Vec<SubscriptionEventFilter>,
) -> bool {
    use zend_common::api;
    macro_rules! match_event {
        ($i:ident) => {
            let_is!(ApiClientEvent::$i = event)
        };
        ($i:ident($p:pat)) => {
            let_is!(ApiClientEvent::$i($p) = event)
        };
    }
    macro_rules! match_message {
        ($i:ident) => {
            match_event!(ApiMessage(api::ServerToClientMessage::$i))
        };
        ($i:ident($p:pat)) => {
            match_event!(ApiMessage(api::ServerToClientMessage::$i($p)))
        };
    }
    filters.iter().any(|filter| match filter {
        SubscriptionEventFilter::Any => true,

        SubscriptionEventFilter::ApiMethodCallReturn(Some(filter_call_id)) => match event {
            ApiClientEvent::ApiMessage(api::ServerToClientMessage::MethodCallReturn(
                api::MethodCallReturn { call_id, .. },
            )) => filter_call_id == call_id,
            _ => false,
        },

        SubscriptionEventFilter::ApiSubscriptionData(Some(filter_sub_id)) => match event {
            ApiClientEvent::ApiMessage(api::ServerToClientMessage::SubscriptionData(
                api::SubscriptionData {
                    subscription_id, ..
                },
            )) => filter_sub_id == subscription_id,
            _ => false,
        },

        SubscriptionEventFilter::ApiMethodCallReturn(None) => {
            match_message!(MethodCallReturn(_))
        }
        SubscriptionEventFilter::ApiSubscriptionData(None) => {
            match_message!(SubscriptionData(_))
        }
        SubscriptionEventFilter::ApiPong => {
            match_message!(Pong)
        }
        SubscriptionEventFilter::ApiInfo => {
            match_message!(Info(_))
        }

        SubscriptionEventFilter::Connected => {
            match_event!(Connected)
        }
        SubscriptionEventFilter::Reconnecting => {
            match_event!(Reconnecting(_))
        }
        SubscriptionEventFilter::Ended => {
            match_event!(Ended)
        }
    })
}
