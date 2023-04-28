use crate::peer_api;
use futures::StreamExt;
use std::{fmt::Display, rc::Rc};
use w::console_log;
use worker as w;
use zend_common::api;

pub trait WebSocketExt {
    /** (n)o (f)ail (send) (j)son, given a less-than-readable name as it's
    frequently used in places with already busy syntax  */
    fn nfsendj<T: serde::Serialize>(&self, data: &T);
    /** (n)o (f)ail (send) (j)son + unwrap, given a less-than-readable name as it's
    frequently used in places with already busy syntax  */
    fn nfsendj_unwrap<T: serde::Serialize, U: Display>(&self, data: &Result<T, U>);
}
impl WebSocketExt for w::WebSocket {
    fn nfsendj<T: serde::Serialize>(&self, data: &T) {
        match serde_json::to_string(data) {
            Ok(json) => match self.send_with_str(json) {
                Ok(_) => console_log!("Successfully sent a message."),
                Err(err) => console_log!("Failed to send a message. {}", err),
            },
            Err(err) => console_log!("Failed to serialise a message. {}", err),
        }
    }
    fn nfsendj_unwrap<T: serde::Serialize, U: Display>(&self, result: &Result<T, U>) {
        match result {
            Ok(data) => self.nfsendj(data),
            Err(err) => console_log!("Failed to unwrap a result. {}", err),
        }
    }
}

#[derive(Debug)]
enum CheckSignedMethodCallError {
    WorkerError(w::Error),
    CheckFail,
}
impl From<w::Error> for CheckSignedMethodCallError {
    fn from(value: w::Error) -> Self {
        Self::WorkerError(value)
    }
}
impl From<()> for CheckSignedMethodCallError {
    fn from(_: ()) -> Self {
        Self::CheckFail
    }
}
async fn check_signed_method_call(
    env: &w::Env,
    signed_call: &api::SignedMethodCall,
) -> Result<(), CheckSignedMethodCallError> {
    if let Err(err) = signed_call.validate_signature() {
        console_log!("Call signature validation failed. {}", err);
        return Err(().into());
    }
    let current_time_secs = w::Date::now().as_millis() / 1000;
    if !signed_call.validate_timestamp(current_time_secs) {
        console_log!("Call timestamp validation failed.");
        return Err(().into());
    }
    let peer = env
        .durable_object("PEER")?
        .id_from_name(
            &signed_call
                .signed_call
                .call
                .common_arguments
                .ecdsa_public_key
                .to_string(),
        )?
        .get_stub()?;
    let mut response = peer
        .fetch_with_request(peer_api::make_request(
            &peer_api::ToPeerMessage::CheckNonceIsUsed(peer_api::CheckNonceMessage {
                nonce: signed_call.signed_call.call.common_arguments.nonce,
            }),
        )?)
        .await?;
    let is_used: bool =
        serde_json::from_str(&response.text().await?).map_err(Into::<w::Error>::into)?;
    if is_used {
        return Err(().into());
    }
    return Ok(());
}

async fn handle_signed_method_call(
    env: Rc<w::Env>,
    signed_call: api::SignedMethodCall,
    server: Rc<w::WebSocket>,
) -> Result<(), ()> {
    if let Err(e) = check_signed_method_call(env.as_ref(), &signed_call).await {
        console_log!("Error when checking signed method call: {:?}", e);
        server.nfsendj(&api::ServerToClientMessage::call_error(
            signed_call.call_id,
            api::ErrorId::InvalidSignature,
            None,
        ));
        return Err(());
    }

    use crate::websocket_api_handlers as h;
    use api::MethodCallArgsVariants as Method;
    let common_args = signed_call.signed_call.call.common_arguments;
    let variant_args = signed_call.signed_call.call.variant_arguments;
    let result = match variant_args {
        Method::CreateRoom => h::create_room(env, common_args).await,
        Method::SubscribeToRoom(args) => {
            h::subscribe_to_room(env, server.clone(), common_args, args).await
        }
        Method::UnsubscribeFromRoom(_) => h::unsubscribe_from_room().await,
        Method::AddPrivilegedPeer(args) => {
            h::add_privileged_peer(env.as_ref(), common_args, args).await
        }
        Method::GetRoomDataHistory(_) => h::get_room_data_history().await,
        Method::DeleteData(_) => h::delete_data().await,
        Method::BroadcastData(args) => h::broadcast_data(env.as_ref(), common_args, args).await,
        Method::UnicastData(_) => h::unicast_data().await,
    };
    let to_send = match result {
        Ok(result) => api::ServerToClientMessage::from_success(signed_call.call_id, result),
        Err(err) => match err {
            h::Error::WorkerError(err) => {
                console_log!("An internal error occured: {}", err);
                api::ServerToClientMessage::from_error(
                    signed_call.call_id,
                    api::ErrorId::InternalError.with_default_message(),
                )
            }
            h::Error::MethodError(err) => {
                api::ServerToClientMessage::from_error(signed_call.call_id, err)
            }
        },
    };
    server.nfsendj(&to_send);
    Ok(())
}

async fn handle_parsed_message(
    env: Rc<w::Env>,
    message: api::ClientToServerMessage,
    server: Rc<w::WebSocket>,
) {
    console_log!("{:?}", message);
    match message {
        api::ClientToServerMessage::Ping => {
            server.nfsendj(&api::ServerToClientMessage::pong());
        }
        api::ClientToServerMessage::SignedMethodCall(signed_call) => match signed_call {
            api::SignedMethodCallOrPartial::Partial(call_id) => {
                server.nfsendj(&api::ServerToClientMessage::from_error(
                    call_id,
                    api::ErrorId::ParseError.with_default_message(),
                ))
            }
            api::SignedMethodCallOrPartial::Full(signed_call) => {
                let _ = handle_signed_method_call(env, signed_call, server).await;
            }
        },
    }
}

async fn handle_message(env: Rc<w::Env>, text: String, server: Rc<w::WebSocket>) {
    // console_log!("{:?}", text);
    match serde_json::from_str::<api::ClientToServerMessage>(&text) {
        Ok(message) => handle_parsed_message(env, message, server).await,
        Err(err) => {
            server.nfsendj(&api::ServerToClientMessage::info(
                "A message failed to be parsed.",
            ));
            console_log!("Failed to parse a message. {}", err);
        }
    }
}

pub async fn handle_ws_server(env: w::Env, server: w::WebSocket) {
    let server = Rc::new(server);
    let env = Rc::new(env);

    let mut event_stream = match server.events() {
        Ok(stream) => stream,
        Err(err) => {
            console_log!("Could not open a websocket stream: {}", err);
            return;
        }
    };

    while let Some(result) = event_stream.next().await {
        let event = match result {
            Err(err) => {
                console_log!(
                    "{} - Error in websocket: {}",
                    w::Date::now().as_millis(),
                    err
                );
                break;
            }
            Ok(event) => event,
        };
        let message_event = match event {
            w::WebsocketEvent::Close(event) => {
                console_log!("{} - {:#?}", w::Date::now().as_millis(), event);
                break;
            }
            w::WebsocketEvent::Message(message_event) => message_event,
        };
        match message_event.text() {
            None => console_log!("no text :("),
            Some(text) => w::wasm_bindgen_futures::spawn_local(handle_message(
                env.clone(),
                text,
                server.clone(),
            )),
        }
    }
    console_log!("closed :)");
}
