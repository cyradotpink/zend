use crate::util::*;
use futures::{channel::mpsc, future, stream::StreamExt};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
use web_sys::WebSocket;
use ws_stream_wasm::{WsMessage, WsMeta, WsStream};
use zend_common::{api, log};

#[derive(Debug, Clone)]
pub enum ApiClientEvent {
    Connected,
    Reconnecting(u64),
    ApiMessage(api::ServerToClientMessage),
    Ended,
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
enum SubscriptionEventFilterItem {
    Any,
    Connected,
    Reconnecting,
    ApiMethodCallReturn(Option<u64>), // Optionally specify call ID
    ApiSubscriptionData(Option<u64>), // Optionally specify subscription ID
    ApiPong,
    ApiInfo,
    Ended,
}
impl Into<Vec<Self>> for SubscriptionEventFilterItem {
    fn into(self) -> Vec<Self> {
        vec![self]
    }
}
pub struct SubscriptionEventFilter {
    inner: Vec<SubscriptionEventFilterItem>,
}

// Overkill but I felt like writing a funny little macro 👍
macro_rules! add_filter_fn {
    ($i:ident, $j:ident $(($e:expr))? $(,$k:ident: $t:ty)*) => {
        pub fn $i(self, $($k: $t,)*) -> Self {
            self.add_filter_item(SubscriptionEventFilterItem::$j$(($e))?)
        }
    };
}
#[allow(dead_code)]
impl SubscriptionEventFilter {
    fn add_filter_item(mut self, item: SubscriptionEventFilterItem) -> Self {
        if self
            .inner
            .iter()
            .any(|v| *v == item || *v == SubscriptionEventFilterItem::Any)
        {
            return self;
        }
        self.inner.push(item);
        self
    }
    pub fn new() -> Self {
        Self { inner: vec![] }
    }
    pub fn any(mut self) -> Self {
        self.inner.clear();
        self.add_filter_item(SubscriptionEventFilterItem::Any)
    }
    add_filter_fn!(call_return_for_id, ApiMethodCallReturn(Some(id)), id: u64);
    add_filter_fn!(sub_data_for_id, ApiSubscriptionData(Some(id)), id: u64);
    add_filter_fn!(connected, Connected);
    add_filter_fn!(reconnecting, Reconnecting);
    add_filter_fn!(call_return, ApiMethodCallReturn(None));
    add_filter_fn!(sub_data, ApiSubscriptionData(None));
    add_filter_fn!(pong, ApiPong);
    add_filter_fn!(info, ApiInfo);
    add_filter_fn!(ended, Ended);
}

#[derive(Debug)]
pub struct EventSubscriptionHandle {
    pub receiver: mpsc::Receiver<ApiClientEvent>,
    id: usize,
    api_client: WsApiClient,
}
impl Drop for EventSubscriptionHandle {
    fn drop(&mut self) {
        self.api_client.unregister_event_subscription(self.id);
    }
}

#[derive(Debug)]
pub enum AwaitEventError {
    Timeout,
    EventsEmpty,
}
#[derive(Debug)]
pub struct AwaitEventHandle {
    receiver: mpsc::Receiver<ApiClientEvent>,
    id: usize,
    api_client: WsApiClient,
    timeout: Option<Duration>,
}
impl AwaitEventHandle {
    pub async fn await_event(mut self) -> Result<ApiClientEvent, AwaitEventError> {
        // zend_common::debug_log_pretty!(self);
        let timeout = match self.timeout {
            Some(v) => v,
            None => {
                return self
                    .receiver
                    .next()
                    .await
                    .ok_or(AwaitEventError::EventsEmpty)
            }
        };
        match future_or_timeout(self.receiver.next(), timeout).await {
            Some(v) => v.ok_or(AwaitEventError::EventsEmpty),
            None => Err(AwaitEventError::Timeout),
        }
    }
}
impl Drop for AwaitEventHandle {
    fn drop(&mut self) {
        self.api_client.unregister_event_subscription(self.id);
    }
}

#[derive(Debug)]
struct WsApiClientInner {
    ws: WsRefCellWrap,
    event_subscriptions: RefCell<Vec<EventSubscription>>,
    next_event_subscription_id: Cell<usize>,
    ws_state: Cell<WebSocketState>,
    clones: Cell<usize>,
}

#[derive(Debug)]
pub struct WsApiClient {
    inner: Rc<WsApiClientInner>,
    anon: bool,
}

// Public Api
#[allow(dead_code)]
impl WsApiClient {
    pub fn new(url: &str) -> Self {
        let event_subscriptions = RefCell::new(Vec::<EventSubscription>::new());
        let ws = WsRefCellWrap::new(url, Some(Duration::from_secs(30)));
        let ws_state = Cell::new(WebSocketState::Reconnecting);
        let next_event_subscription_id = Cell::new(0usize);
        let data = WsApiClientInner {
            ws,
            event_subscriptions,
            next_event_subscription_id,
            ws_state,
            clones: Cell::new(1),
        };
        let new_client = Self {
            inner: Rc::new(data),
            anon: false,
        };
        // These clones are "anonymous" because they don't count towards the "clones" counter
        // in inner.
        let client = new_client.anon_clone();
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(event) = client.inner.ws.next_event().await {
                handle_event(event, &client);
            }
            client
                .inner
                .event_subscriptions
                .borrow_mut()
                .iter_mut()
                .for_each(|v| {
                    v.sender.close_channel();
                });
            log!("event handler task ended");
        });
        let client = new_client.anon_clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                match client.await_state(WebSocketState::Connected).await {
                    Err(_) => break, // Ws ended and will never connect again
                    _ => {
                        zend_common::log!()
                    } // Ws was already connected or became connected after some time
                }
                let _ = client.send_message(&api::ClientToServerMessage::Ping);
                zend_common::log!();

                match client
                    .await_state_with_timeout(WebSocketState::Reconnecting, Duration::from_secs(10))
                    .await
                {
                    Ok(_) => continue, // Ws entered reconnecting state
                    Err(e) => match e {
                        AwaitEventError::Timeout => continue,  // Ws is still connected
                        AwaitEventError::EventsEmpty => break, // Ws will never connect again
                    },
                };
            }
            log!("pinger task ended");
        });
        new_client
    }

    pub fn end(&self) {
        self.inner.ws.end();
    }

    pub fn send_message(&self, message: &api::ClientToServerMessage) -> Result<(), ()> {
        let message = match serde_json::to_string(message) {
            Ok(v) => v,
            Err(_) => return Err(()),
        };
        self.inner.ws.send(&message);
        return Ok(());
    }

    pub fn get_event_handle(&self, filter: SubscriptionEventFilter) -> AwaitEventHandle {
        let (id, receiver) =
            self.register_event_subscription(EventSubscriptionType::Once, filter.inner);
        AwaitEventHandle {
            receiver,
            id,
            api_client: self.anon_clone(),
            timeout: None,
        }
    }

    pub fn get_event_handle_timeout(
        &self,
        filter: SubscriptionEventFilter,
        timeout: Duration,
    ) -> AwaitEventHandle {
        let (id, receiver) =
            self.register_event_subscription(EventSubscriptionType::Once, filter.inner);
        AwaitEventHandle {
            receiver,
            id,
            api_client: self.anon_clone(),
            timeout: Some(timeout),
        }
    }

    pub fn receive_events(&self, filter: SubscriptionEventFilter) -> EventSubscriptionHandle {
        let (id, receiver) =
            self.register_event_subscription(EventSubscriptionType::Persistent, filter.inner);
        EventSubscriptionHandle {
            receiver,
            id,
            api_client: self.anon_clone(),
        }
    }
}

// Implementation Details
impl WsApiClient {
    fn anon_clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            anon: true,
        }
    }

    fn register_event_subscription(
        &self,
        subscriber_type: EventSubscriptionType,
        event_filters: Vec<SubscriptionEventFilterItem>,
    ) -> (usize, mpsc::Receiver<ApiClientEvent>) {
        let (mut sender, receiver) = mpsc::channel::<ApiClientEvent>(256);
        let id_cell = &self.inner.next_event_subscription_id;
        let id = id_cell.get();
        if self.inner.clones.get() < 1 {
            sender.close_channel();
            return (id, receiver);
        }
        self.inner
            .event_subscriptions
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

    fn unregister_event_subscription(&self, id: usize) {
        let mut subscriptions = self.inner.event_subscriptions.borrow_mut();
        let index = match subscriptions.iter().position(|v| v.id == id) {
            Some(v) => v,
            _ => return,
        };
        subscriptions.swap_remove(index);
    }

    fn await_state_common(&self, states: Vec<WebSocketState>) -> Option<SubscriptionEventFilter> {
        let current_state = self.inner.ws_state.get();
        if states.iter().any(|v| *v == current_state) {
            return None;
        }
        Some(SubscriptionEventFilter {
            inner: states
                .into_iter()
                .map(|v| match v {
                    WebSocketState::Connected => SubscriptionEventFilterItem::Connected,
                    WebSocketState::Reconnecting => SubscriptionEventFilterItem::Reconnecting,
                    WebSocketState::Ended => SubscriptionEventFilterItem::Ended,
                })
                .collect(),
        })
    }

    async fn await_state<T: Into<Vec<WebSocketState>>>(&self, states: T) -> Result<(), ()> {
        match self.await_state_common(states.into()) {
            Some(state_filter) => self
                .get_event_handle(state_filter)
                .await_event()
                .await
                .map(|_| ())
                .map_err(|_| ()),
            None => Ok(()),
        }
    }

    async fn await_state_with_timeout<T: Into<Vec<WebSocketState>>>(
        &self,
        states: T,
        timeout: Duration,
    ) -> Result<(), AwaitEventError> {
        match self.await_state_common(states.into()) {
            Some(state_filter) => self
                .get_event_handle_timeout(state_filter, timeout)
                .await_event()
                .await
                .map(|_| ()),
            None => Ok(()),
        }
    }
}

impl Clone for WsApiClient {
    fn clone(&self) -> Self {
        let clones = self.inner.clones.get();
        self.inner.clones.set(clones + 1);
        Self {
            inner: Rc::clone(&self.inner),
            anon: false,
        }
    }
}

impl Drop for WsApiClient {
    fn drop(&mut self) {
        if self.anon {
            return;
        }
        let clones = self.inner.clones.get();
        if clones <= 1 {
            log!("hi its me the wsapiclient drop glue");
            self.end();
        }
        self.inner.clones.set(clones - 1);
    }
}

fn handle_event(event: WrappedSocketEvent, client: &WsApiClient) {
    let event = {
        use WrappedSocketEvent::*;
        match event {
            Connected => {
                client.inner.ws_state.set(WebSocketState::Connected);
                ApiClientEvent::Connected
            }
            Reconnecting(v) => {
                client.inner.ws_state.set(WebSocketState::Reconnecting);
                ApiClientEvent::Reconnecting(v)
            }
            Ended(_) => {
                client.inner.ws_state.set(WebSocketState::Ended);
                ApiClientEvent::Ended
            }

            TextMessage(msg) => ApiClientEvent::ApiMessage(match serde_json::from_str(&msg) {
                Ok(v) => v,
                Err(_) => return,
            }),
            BinaryMessage(_) => return,
        }
    };
    // Ref only held until end of loop iteration, before which no .await occurs
    let mut subscribers = client.inner.event_subscriptions.borrow_mut();
    let mut i = 0;
    loop {
        if i >= subscribers.len() {
            break;
        }
        let subscriber = subscribers
            .get_mut(i)
            .expect("Subscribers list bounds check failed during get");
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
    filters: &Vec<SubscriptionEventFilterItem>,
) -> bool {
    macro_rules! let_is {
        ($p:pat = $i:ident) => {
            if let $p = $i {
                true
            } else {
                false
            }
        };
    }
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
    use SubscriptionEventFilterItem::*;
    filters.iter().any(|filter| match filter {
        Any => true,

        ApiMethodCallReturn(Some(filter_call_id)) => match event {
            ApiClientEvent::ApiMessage(api::ServerToClientMessage::MethodCallReturn(
                api::MethodCallReturn { call_id, .. },
            )) => filter_call_id == call_id,
            _ => false,
        },

        ApiSubscriptionData(Some(filter_sub_id)) => match event {
            ApiClientEvent::ApiMessage(api::ServerToClientMessage::SubscriptionData(
                api::SubscriptionData {
                    subscription_id, ..
                },
            )) => filter_sub_id == subscription_id,
            _ => false,
        },

        ApiMethodCallReturn(None) => {
            match_message!(MethodCallReturn(_))
        }
        ApiSubscriptionData(None) => {
            match_message!(SubscriptionData(_))
        }
        ApiPong => {
            match_message!(Pong)
        }
        ApiInfo => {
            match_message!(Info(_))
        }

        Connected => {
            match_event!(Connected)
        }
        Reconnecting => {
            match_event!(Reconnecting(_))
        }
        Ended => {
            match_event!(Ended)
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebSocketState {
    Connected,
    Reconnecting,
    Ended,
}
impl Into<Vec<Self>> for WebSocketState {
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
    event_filters: Vec<SubscriptionEventFilterItem>,
    sender: mpsc::Sender<ApiClientEvent>,
    subscriber_type: EventSubscriptionType,
    id: usize,
}

#[derive(Debug)]
enum WrappedSocketEvent {
    Connected,
    // Seconds until next reconnection attempt
    Reconnecting(u64),
    TextMessage(String),
    BinaryMessage(Vec<u8>),
    Ended(&'static str),
}

#[derive(Debug)]
struct WebSocketWrap {
    finished: bool,
    url: String,
    ws: Option<WsStream>,
    retry_after: u64,
    close_timeout: Duration,
}
impl WebSocketWrap {
    fn new(url: &str, close_timeout: Option<Duration>) -> Self {
        Self {
            finished: false,
            url: url.into(),
            ws: None,
            retry_after: 0,
            close_timeout: close_timeout.unwrap_or(Duration::MAX),
        }
    }

    async fn connect(&mut self) -> Result<WsStream, &'static str> {
        let connect_future = Box::pin(WsMeta::connect(&self.url, None));
        let timeout_future = gloo_timers::future::sleep(Duration::from_secs(5));
        let select = future::select(connect_future, timeout_future).await;
        let (_, wsio) = match select {
            future::Either::Left((value, _)) => value.map_err(|_| "WsErr")?,
            future::Either::Right(_) => return Err("Timeout"),
        };
        Ok(wsio)
    }

    async fn next_event(&mut self) -> Option<WrappedSocketEvent> {
        if self.finished {
            return None;
        }
        if let Some(wsio) = &mut self.ws {
            let timeout_future = gloo_timers::future::sleep(self.close_timeout);
            let next_result = match future::select(wsio.next(), timeout_future).await {
                future::Either::Left((v, _)) => v,
                future::Either::Right(_) => {
                    if let Some(wsio) = self.ws.take() {
                        wsio.wrapped()
                            .close()
                            .expect("Something went wrong when closing a websocket connection");
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
struct WsRefCellWrap {
    ws_wrap: RefCell<WebSocketWrap>,
    ws_copy: RefCell<Option<WebSocket>>,
    ended: Cell<bool>,
    end_channel: (RefCell<mpsc::Sender<()>>, RefCell<mpsc::Receiver<()>>),
}
impl WsRefCellWrap {
    fn new(url: &str, close_timeout: Option<Duration>) -> Self {
        let (sender, receiver) = mpsc::channel(0);
        Self {
            ws_wrap: RefCell::new(WebSocketWrap::new(url, close_timeout)),
            ws_copy: RefCell::new(None),
            ended: Cell::new(false),
            end_channel: (RefCell::new(sender), RefCell::new(receiver)),
        }
    }
    fn end(&self) {
        let _ = self.end_channel.0.borrow_mut().try_send(());
    }
    fn send(&self, s: &str) {
        let ws = self.ws_copy.borrow();
        if let Some(ref ws) = *ws {
            let _ = ws.send_with_str(s);
        }
    }
    async fn next_event(&self) -> Option<WrappedSocketEvent> {
        if self.ended.get() {
            return None;
        }
        let mut wrap = self
            .ws_wrap
            .try_borrow_mut()
            .expect("You ran next_event() twice at the same time. Don't do that :(");

        let mut recv = self.end_channel.1.borrow_mut();
        let next_event_future = Box::pin(wrap.next_event());
        let end_future = recv.next();
        let event = match future::select(next_event_future, end_future).await {
            future::Either::Left((ev, _)) => ev?,
            future::Either::Right(_) => WrappedSocketEvent::Ended("End() called"),
        };
        use WrappedSocketEvent::*;
        match event {
            Connected => {
                let mut ws = self.ws_copy.borrow_mut();
                if let Some(new) = &wrap.ws {
                    let _ = ws.insert(new.wrapped().clone());
                }
            }
            Reconnecting(_) => {
                let mut ws = self.ws_copy.borrow_mut();
                ws.take();
            }
            Ended(_) => {
                self.ended.set(true);
                let ws = self.ws_copy.borrow_mut().take();
                if let Some(ref ws) = ws {
                    let _ = ws.close();
                    wrap.finished = true;
                }
            }
            _ => {}
        }
        Some(event)
    }
}
