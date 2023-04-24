use core::panic;
use std::{cell::RefCell, time::Duration};

use futures::stream::StreamExt;
use pharos::{Events, Observable, ObserveConfig};
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;
use ws_stream_wasm::{WsErr, WsEvent, WsMessage, WsMeta, WsStream};

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
    Message(String),
    Disconnected(String),
}
struct WebSocketWrap {
    finished: bool,
    url: String,
    ws: Option<(WsStream, Events<WsEvent>)>,
    retry_after: u64,
}
impl WebSocketWrap {
    fn new(url: &str) -> Self {
        Self {
            finished: false,
            url: url.into(),
            ws: None,
            retry_after: 0,
        }
    }

    async fn connect(&mut self) -> Result<(WsStream, Events<WsEvent>), WsErr> {
        let (mut ws, wsio) = WsMeta::connect(&self.url, None).await?;
        let events = ws.observe(ObserveConfig::default()).await.unwrap();
        Ok((wsio, events))
    }
    async fn next_event(&mut self) -> Option<WrappedSocketEvent> {
        if self.finished {
            log("finsdf");
            return None;
        }
        if let Some((wsio, events)) = &mut self.ws {
            if let Some(msg) = wsio.next().await {
                if let WsMessage::Text(s) = msg {
                    return Some(WrappedSocketEvent::Message(s));
                } else {
                    return Some(WrappedSocketEvent::Message("".to_string()));
                }
            }
            let close_event = loop {
                match events.next().await {
                    Some(WsEvent::Closed(ev)) => break ev,
                    Some(_) => continue,
                    None => {
                        self.finished = true;
                        return Some(WrappedSocketEvent::Disconnected("Unreachable".to_string()));
                    }
                }
            };
            if close_event.was_clean {
                self.finished = true;
                return Some(WrappedSocketEvent::Disconnected("Clean".to_string()));
            }
            self.ws = None;
            return Some(WrappedSocketEvent::Reconnecting(self.retry_after));
        }
        if self.retry_after > 0 {
            gloo_timers::future::sleep(Duration::from_secs(self.retry_after)).await;
            // Exponential back-off maxing out at 60 seconds
            self.retry_after = if self.retry_after * 2 > 60 {
                60
            } else {
                self.retry_after * 2
            };
        } else {
            self.retry_after = 5;
        }
        if let Ok(new) = self.connect().await {
            self.retry_after = 0;
            self.ws = Some(new);
            return Some(WrappedSocketEvent::Connected);
        }
        Some(WrappedSocketEvent::Reconnecting(self.retry_after))
    }
}

// At least I'm keeping the refcell crimes contained in their own little refcell crimes struct
pub struct MutWrap {
    ws_wrap: RefCell<WebSocketWrap>,
    // References never held across awaits, therefore "safe" to mutate
    ws_copy: RefCell<Option<WebSocket>>,
}
impl MutWrap {
    pub fn new(url: &str) -> Self {
        Self {
            ws_wrap: RefCell::new(WebSocketWrap::new(url)),
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
        let mut wrap = self.ws_wrap.borrow_mut();
        let event = wrap.next_event().await?;
        match event {
            WrappedSocketEvent::Connected => {
                let mut ws = self.ws_copy.borrow_mut();
                if let Some(new) = &wrap.ws {
                    let _ = ws.insert(new.0.wrapped().clone());
                }
            }
            WrappedSocketEvent::Disconnected(_) => {
                let mut ws = self.ws_copy.borrow_mut();
                ws.take();
            }
            _ => {}
        }
        Some(event)
    }
}
