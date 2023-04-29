use std::{
    cell::{Cell, RefCell},
    ops::Deref,
    rc::Rc,
    time::Duration,
};

use futures::{channel::mpsc, future, stream::StreamExt};
use pharos::{Events, Observable, ObserveConfig};
use std::future::Future;
use wasm_bindgen::prelude::{wasm_bindgen, UnwrapThrowExt};
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

/*
async fn future_or_timeout<A>(future: A, timeout: Duration) -> Result<A::Output, ()>
where
    A: Future + Unpin,
{
    let timeout_fut = gloo_timers::future::sleep(timeout);
    match futures::future::select(future, timeout_fut).await {
        futures::future::Either::Left((v, _)) => Ok(v),
        futures::future::Either::Right(_) => Err(()),
    }
}
pub enum TimeoutOrPassedError<E> {
    Timeout,
    Passed(E),
}
async fn result_future_or_timeout<A, T, E>(
    future: A,
    timeout: Duration,
) -> Result<T, TimeoutOrPassedError<E>>
where
    A: Future<Output = Result<T, E>> + Unpin,
{
    match future_or_timeout(future, timeout).await {
        Ok(v) => match v {
            Ok(v) => Ok(v),
            Err(e) => Err(TimeoutOrPassedError::Passed(e)),
        },
        Err(_) => Err(TimeoutOrPassedError::Timeout),
    }
}*/

#[derive(Debug)]
pub enum WrappedSocketEvent {
    Connected,
    /// Seconds until next reconnection attempt
    Reconnecting(u64),
    TextMessage(String),
    BinaryMessage(Vec<u8>),
    Ended(&'static str),
}

#[derive(Debug)]
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

#[derive(Debug)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketState {
    Connected,
    Reconnecting,
    Ended,
}
impl Into<Vec<Self>> for WebSocketState {
    fn into(self) -> Vec<Self> {
        vec![self]
    }
}

#[derive(Debug, Clone)]
pub enum ApiClientEvent {
    Connected,
    Reconnecting(u64),
    ApiMessage(zend_common::api::ServerToClientMessage),
    Ended,
}

#[allow(unused)]
#[derive(Debug)]
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

#[derive(Debug)]
enum EventSubscriptionType {
    Once,
    Persistent,
}

#[derive(Debug)]
struct EventSubscription {
    event_filters: Vec<SubscriptionEventFilter>,
    sender: mpsc::Sender<ApiClientEvent>,
    subscriber_type: EventSubscriptionType,
    id: u64,
}

pub struct EventSubscriptionHandle {
    pub receiver: mpsc::Receiver<ApiClientEvent>,
    id: u64,
    api_client: WsApiClient,
}
impl EventSubscriptionHandle {
    pub fn unsubscribe(self) {
        drop(self)
    }
}
impl Drop for EventSubscriptionHandle {
    fn drop(&mut self) {
        self.api_client.unregister_event_subscription(self.id);
    }
}

#[derive(Debug)]
pub enum TimeoutOrEndedError {
    Timeout,
    Ended,
}

#[derive(Debug)]
pub struct WsApiClientFields {
    ws: WsRefCellWrap,
    event_subscriptions: RefCell<Vec<EventSubscription>>,
    next_event_subscription_id: Cell<u64>,
    ws_state: Cell<WebSocketState>,
}

// TODO perform cleanup when last reference to WsApiClientFields is dropped (-> End background tasks)
#[derive(Debug, Clone)]
pub struct WsApiClient(Rc<WsApiClientFields>);
impl WsApiClient {
    pub fn new(url: &str) -> Self {
        let event_subscriptions = RefCell::new(Vec::<EventSubscription>::new());
        let ws = WsRefCellWrap::new(url, Some(Duration::from_secs(30)));
        let ws_state = Cell::new(WebSocketState::Reconnecting);
        let next_event_subscription_id = Cell::new(0u64);
        let data = WsApiClientFields {
            ws,
            event_subscriptions,
            next_event_subscription_id,
            ws_state,
        };
        let client = Self(Rc::new(data));
        let client_clone = client.clone();

        wasm_bindgen_futures::spawn_local(async move {
            while let Some(event) = client.ws.next_event().await {
                handle_event(event, &client);
            }
            client
                .0
                .event_subscriptions
                .borrow_mut()
                .iter_mut()
                .for_each(|v| v.sender.close_channel())
        });
        let client = client_clone;
        let client_clone = client.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                match client.await_state(WebSocketState::Connected).await {
                    Err(_) => break, // Ws ended and will never connect again
                    _ => {}          // Ws was already connected or became connected after some time
                }
                let _ = client.send_message(&api::ClientToServerMessage::Ping);

                match client
                    .await_state_with_timeout(WebSocketState::Reconnecting, Duration::from_secs(10))
                    .await
                {
                    Ok(_) => continue, // Ws entered reconnecting state
                    Err(e) => match e {
                        TimeoutOrEndedError::Timeout => continue, // Ws is still connected
                        TimeoutOrEndedError::Ended => break,      // Ws will never connect again
                    },
                };
            }
        });
        client_clone
    }

    pub fn send_message(&self, message: &api::ClientToServerMessage) -> Result<(), ()> {
        let message = match serde_json::to_string(message) {
            Ok(v) => v,
            Err(_) => return Err(()),
        };
        self.ws.send(&message);
        return Ok(());
    }

    fn register_event_subscription(
        &self,
        subscriber_type: EventSubscriptionType,
        event_filters: Vec<SubscriptionEventFilter>,
    ) -> (u64, mpsc::Receiver<ApiClientEvent>) {
        let (sender, receiver) = mpsc::channel::<ApiClientEvent>(256);
        let id_cell = &self.next_event_subscription_id;
        let id = id_cell.get();
        self.event_subscriptions
            .borrow_mut()
            .push(EventSubscription {
                event_filters,
                sender,
                subscriber_type,
                id,
            });
        id_cell.set(id + 1);
        (id, receiver)
    }

    pub fn unregister_event_subscription(&self, id: u64) {
        let mut subscriptions = self.event_subscriptions.borrow_mut();
        let index = match subscriptions.iter().position(|v| v.id == id) {
            Some(v) => v,
            _ => return,
        };
        subscriptions.swap_remove(index);
    }

    pub fn one_event_future<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
    ) -> (u64, impl Future<Output = Result<ApiClientEvent, ()>>) {
        let (id, mut receiver) =
            self.register_event_subscription(EventSubscriptionType::Once, filters.into());
        let fut = async move { receiver.next().await.ok_or(()) };
        (id, fut)
    }

    pub async fn await_one_event<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
    ) -> Result<ApiClientEvent, ()> {
        self.one_event_future(filters).1.await
    }

    pub async fn await_event_with_timeout<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
        timeout: Duration,
    ) -> Result<ApiClientEvent, TimeoutOrEndedError> {
        let timeout_fut = gloo_timers::future::sleep(timeout);
        let (sub_id, event_future) = self.one_event_future(filters);
        let event_future = Box::pin(event_future);
        match future::select(event_future, timeout_fut).await {
            future::Either::Left((v, _)) => return v.map_err(|_| TimeoutOrEndedError::Ended),
            _ => {}
        }
        self.unregister_event_subscription(sub_id);
        Err(TimeoutOrEndedError::Timeout)
    }

    fn await_state_common(
        &self,
        states: Vec<WebSocketState>,
    ) -> Option<Vec<SubscriptionEventFilter>> {
        let current_state = self.ws_state.get();
        if states.iter().any(|v| *v == current_state) {
            return None;
        }
        drop(current_state);
        Some(
            states
                .into_iter()
                .map(|v| match v {
                    WebSocketState::Connected => SubscriptionEventFilter::Connected,
                    WebSocketState::Reconnecting => SubscriptionEventFilter::Reconnecting,
                    WebSocketState::Ended => SubscriptionEventFilter::Ended,
                })
                .collect(),
        )
    }

    pub async fn await_state<T: Into<Vec<WebSocketState>>>(&self, states: T) -> Result<(), ()> {
        match self.await_state_common(states.into()) {
            Some(state_filter) => self.await_one_event(state_filter).await.map(|_| ()),
            None => Ok(()),
        }
    }

    pub async fn await_state_with_timeout<T: Into<Vec<WebSocketState>>>(
        &self,
        states: T,
        timeout: Duration,
    ) -> Result<(), TimeoutOrEndedError> {
        match self.await_state_common(states.into()) {
            Some(state_filter) => self
                .await_event_with_timeout(state_filter, timeout)
                .await
                .map(|_| ()),
            None => Ok(()),
        }
    }

    pub fn receive_events<T: Into<Vec<SubscriptionEventFilter>>>(
        &self,
        filters: T,
    ) -> EventSubscriptionHandle {
        let (id, receiver) =
            self.register_event_subscription(EventSubscriptionType::Persistent, filters.into());
        EventSubscriptionHandle {
            receiver,
            id,
            api_client: self.clone(),
        }
    }
}
impl Deref for WsApiClient {
    type Target = Rc<WsApiClientFields>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn handle_event(event: WrappedSocketEvent, client: &WsApiClient) {
    let event = match event {
        WrappedSocketEvent::Connected => {
            client.ws_state.set(WebSocketState::Connected);
            ApiClientEvent::Connected
        }
        WrappedSocketEvent::Reconnecting(v) => {
            client.ws_state.set(WebSocketState::Reconnecting);
            ApiClientEvent::Reconnecting(v)
        }
        WrappedSocketEvent::Ended(_) => {
            client.ws_state.set(WebSocketState::Ended);
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
    let mut subscribers = client.event_subscriptions.borrow_mut();
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
