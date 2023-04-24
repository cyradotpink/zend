use std::{cell::RefCell, time::Duration};

use futures::{future, stream::StreamExt};
use pharos::{Events, Observable, ObserveConfig};
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
use ws_stream_wasm::{WsEvent, WsMessage, WsMeta, WsStream};

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
    Disconnected(&'static str),
}
struct WebSocketWrap {
    finished: bool,
    url: String,
    ws: Option<(WsStream, Events<WsEvent>)>,
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
    async fn next_event(&mut self) -> Option<WrappedSocketEvent> {
        if self.finished {
            return None;
        }
        if let Some((wsio, events)) = &mut self.ws {
            // let next_future = wsio.next();
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
            let close_event = loop {
                match events.next().await {
                    Some(WsEvent::Closed(ev)) => break ev,
                    Some(_) => continue,
                    None => {
                        self.finished = true;
                        return Some(WrappedSocketEvent::Disconnected("Unreachable code reached"));
                    }
                }
            };
            if close_event.was_clean {
                self.finished = true;
                return Some(WrappedSocketEvent::Disconnected("Clean"));
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
            Err(err) => {
                log(err);
                WrappedSocketEvent::Reconnecting(self.retry_after)
            }
        })
    }
}

// At least I'm keeping the refcell crimes contained in their own little refcell crimes struct
pub struct WsRefCellWrap {
    ws_wrap: RefCell<WebSocketWrap>,
    ws_copy: RefCell<Option<WebSocket>>,
}
impl WsRefCellWrap {
    pub fn new(url: &str, close_timeout: Option<Duration>) -> Self {
        Self {
            ws_wrap: RefCell::new(WebSocketWrap::new(url, close_timeout)),
            ws_copy: RefCell::new(None),
        }
    }
    pub fn send(&self, s: &str) {
        let ws = self.ws_copy.borrow();
        if let Some(ws) = &*ws {
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
