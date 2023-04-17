use crate::{
    peer_api,
    websocket_api::{self as api, SignedMethodCall},
};
use futures::StreamExt;
use serde;
use std::{fmt::Display, rc::Rc};
use w::console_log;
use worker as w;

trait ResultExt<T> {
    fn expect_send(self, server: &w::WebSocket, call_id: u64) -> Result<T, ()>;
}
impl<T, E: Display> ResultExt<T> for Result<T, E> {
    fn expect_send(self, server: &w::WebSocket, call_id: u64) -> Result<T, ()> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => {
                console_log!("Got error. {}", e);
                server.nfsendj(&api::ServerToClientMessage::call_error(
                    call_id,
                    api::ErrorId::InternalError,
                    None,
                ));
                Err(())
            }
        }
    }
}

trait WebSocketExt {
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

mod method_handlers {
    use crate::{
        room_api::{self, IntoRequest},
        w, websocket_api as api,
    };
    use enum_convert::EnumConvert;
    use std::rc::Rc;

    #[derive(EnumConvert)]
    pub enum Error<T: serde::Serialize> {
        #[enum_convert(from)]
        WorkerError(w::Error),
        MethodError(T),
    }
    impl<T: serde::Serialize> From<serde_json::Error> for Error<T> {
        fn from(value: serde_json::Error) -> Self {
            Self::WorkerError(value.into())
        }
    }

    pub async fn create_room(
        env: Rc<w::Env>,
        _: api::CreateRoomArgs,
        common_args: api::MethodCallCommonArgs,
    ) -> Result<api::CreateRoomReturn, Error<()>> {
        let request = room_api::InitialiseMessage {
            initial_peer_id: common_args.ecdsa_public_key,
        }
        .into_request()?;
        let namespace = env.durable_object("ROOM")?;
        let mut room_id: Option<api::RoomId> = None;
        while let None = room_id {
            let tmp_id = api::RoomId::random(w::js_sys::Math::random);
            let tmp_stub = namespace.id_from_name(&tmp_id.to_string())?.get_stub()?;
            //                           // not really sure why the request has to be consumed here but w/e
            let mut response = tmp_stub.fetch_with_request(request.clone()?).await?;
            let success = serde_json::from_str(&response.text().await?)?;
            if success {
                room_id = Some(tmp_id)
            }
        }
        // Reasonable unwrap because the loop condition forces room_id to be Some here
        let room_id = room_id.unwrap();
        Ok(api::CreateRoomReturn { room_id })
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

    /*enum MappedError {
        SerdeJsonError(serde_json::Error),
    }*/
    fn map_json<T: serde::Serialize, U: serde::Serialize>(
        result: Result<T, method_handlers::Error<U>>,
    ) -> Result<Result<serde_json::Value, serde_json::Value>, w::Error> {
        match result {
            Ok(v) => match serde_json::to_value(v) {
                Ok(v) => Ok(Ok(v)), // The Method call was Ok and the returned value serialised Ok
                Err(e) => Err(e.into()), // The method call was Ok but the returned value failed to serialise
            },
            Err(e) => match e {
                h::Error::WorkerError(e) => Err(e), // There was some unexpected error while handling the call
                h::Error::MethodError(e) => match serde_json::to_value(e) {
                    Ok(v) => Ok(Err(v)), // The method call intentionally returned an error and the error serialised Ok
                    Err(e) => Err(e.into()), // The method call intentially returned an error but the error failed to serialise
                },
            },
        }
    }

    use api::MethodCallArgsVariants as Method;
    use method_handlers as h;
    let common_args = signed_call.signed_call.call.common_arguments;
    let variant_args = signed_call.signed_call.call.variant_arguments;
    let result = match variant_args {
        Method::CreateRoom(args) => map_json(h::create_room(env, args, common_args).await),
        Method::SubscribeToRoom(_) => todo!(),
        Method::AddPrivilegedPeer(_) => todo!(),
        Method::GetRoomDataHistory(_) => todo!(),
        Method::DeleteData(_) => todo!(),
        Method::BroadcastData(_) => todo!(),
        Method::UnicastData(_) => todo!(),
    };
    match result {
        Ok(serialize_result) => match serialize_result {
            Ok(value) => server.nfsendj_unwrap(&api::ServerToClientMessage::call_success(
                signed_call.call_id,
                value,
            )),
            Err(err_value) => server.nfsendj(&api::ServerToClientMessage::call_error(
                signed_call.call_id,
                api::ErrorId::InternalError,
                None,
            )),
        },
        Err(err) => console_log!("Internal error. {}", err),
    };
    Ok(())
}

async fn handle_parsed_message(
    env: Rc<w::Env>,
    message: api::ClientToServerMessage,
    server: Rc<w::WebSocket>,
) {
    match message {
        api::ClientToServerMessage::Ping => {
            server.nfsendj(&api::ServerToClientMessage::pong());
        }
        api::ClientToServerMessage::SignedMethodCall(signed_call) => match signed_call {
            api::SignedMethodCallOrPartial::Partial(call_id) => server.nfsendj(
                &api::ServerToClientMessage::call_error(call_id, api::ErrorId::ParseError, None),
            ),
            api::SignedMethodCallOrPartial::Full(signed_call) => {
                let _ = handle_signed_method_call(env, signed_call, server).await;
            }
        },
    }
}

async fn handle_message(env: Rc<w::Env>, text: String, server: Rc<w::WebSocket>) {
    console_log!("{:?}", text);
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

    let mut event_stream = server.events().expect("could not open stream");

    while let Some(result) = event_stream.next().await {
        match result {
            Ok(event) => match event {
                w::WebsocketEvent::Message(msg) => {
                    match msg.text() {
                        Some(text) => w::wasm_bindgen_futures::spawn_local(handle_message(
                            env.clone(),
                            text,
                            server.clone(),
                        )),
                        None => console_log!("no text :("),
                    }
                    console_log!("{} - {:#?}", w::Date::now().as_millis(), msg.text())
                }
                w::WebsocketEvent::Close(event) => {
                    console_log!("{} - {:#?}", w::Date::now().as_millis(), event)
                }
            },
            Err(err) => console_log!(
                "{} - Error in websocket: {}",
                w::Date::now().as_millis(),
                err
            ),
        }
    }
    console_log!("closed :)");
}
