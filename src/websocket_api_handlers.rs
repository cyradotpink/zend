use crate::{
    room_api::{self, IntoRequest},
    util,
    websocket::WebSocketExt,
    websocket_api as api,
};
use async_std::stream::StreamExt;
use enum_convert::EnumConvert;
use serde::Deserialize;
use std::rc::Rc;
use worker::{self as w, console_log};

#[derive(Deserialize)]
struct SubscriptionDataMessage {
    sender_id: api::EcdsaPublicKeyWrapper,
    nonce: api::Nonce,
    data: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "message_type", content = "message_content")]
enum FromRoomMessage {
    Close,
    Data(SubscriptionDataMessage),
}

fn get_room_stub(env: &w::Env, room_id: api::RoomId) -> Result<w::Stub, w::Error> {
    env.durable_object("ROOM")?
        .id_from_name(&room_id.to_string())?
        .get_stub()
}

#[derive(EnumConvert, Debug)]
#[enum_convert(from)]
pub enum Error {
    WorkerError(w::Error),
    MethodError(api::MethodCallError),
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        value.into()
    }
}

pub async fn create_room(
    env: Rc<w::Env>,
    common_args: api::MethodCallCommonArgs,
) -> Result<api::MethodCallSuccess, Error> {
    let namespace = env.durable_object("ROOM")?;
    let mut room_id: Option<api::RoomId> = None;
    while let None = room_id {
        let tmp_id = api::RoomId::from_random(
            util::math_random().map_err(|_| api::ErrorId::InternalError.with_default_message())?,
        );
        let tmp_stub = namespace.id_from_name(&tmp_id.to_string())?.get_stub()?;
        // Create a new request every iteration instead of cloning to save an allocation
        // because in the most likely case, the loop exits after the first iteration
        let request = room_api::InitialiseMessage {
            initial_peer_id: common_args.ecdsa_public_key.clone(),
        }
        .into_request()?;
        let mut response = tmp_stub.fetch_with_request(request).await?;
        let success = serde_json::from_str(&response.text().await?)?;
        if success {
            room_id = Some(tmp_id)
        }
    }
    // Reasonable unwrap because the loop condition forces room_id to be Some here
    let room_id = room_id.unwrap();
    Ok(api::CreateRoomSuccess { room_id }.into())
}

// TODO possibly reconnect to the room object if the connection dies?
// if this turns out to be a rare occurence, this work could be offloaded to the client
async fn subscriber_background_future(
    env: Rc<w::Env>,
    server: Rc<w::WebSocket>,
    common_args: api::MethodCallCommonArgs,
    args: api::SubscribeToRoomArgs,
    subscription_id: u64,
) -> Result<(), Error> {
    let room_id = args.room_id;
    let request = room_api::SubscribeMessage {
        subscriber_id: common_args.ecdsa_public_key,
        subscription_id,
    }
    .into_request()?;
    let stub = get_room_stub(env.as_ref(), room_id)?;
    let response = stub.fetch_with_request(request).await?;
    let ws_client = match response.websocket() {
        Some(ws_client) => ws_client,
        None => {
            return Ok(());
        }
    };
    ws_client.accept()?;
    let mut event_stream = server.events()?;
    while let Some(result) = event_stream.next().await {
        let event = match result {
            Err(err) => {
                console_log!("Error in connection to room: {}", err);
                break;
            }
            Ok(event) => event,
        };
        let message = match event {
            w::WebsocketEvent::Close(event) => {
                console_log!("(Connection to room closed) {:#?}", event);
                break;
            }
            w::WebsocketEvent::Message(message) => message,
        };
        let text = match message.text() {
            None => break,
            Some(text) => text,
        };
        let message = serde_json::from_str::<FromRoomMessage>(&text)?;
        let data_message = match message {
            FromRoomMessage::Close => {
                ws_client.close(None, None::<&str>)?;
                break;
            }
            FromRoomMessage::Data(data_message) => data_message,
        };
        server.nfsendj(
            &api::SubscriptionData {
                subscription_id,
                room_id,
                sender_id: data_message.sender_id,
                nonce: data_message.nonce,
                data: data_message.data,
            }
            .into_message(),
        )
    }
    Ok(())
}
pub async fn subscribe_to_room(
    env: Rc<w::Env>,
    server: Rc<w::WebSocket>,
    common_args: api::MethodCallCommonArgs,
    args: api::SubscribeToRoomArgs,
    subscription_id: u64,
) -> Result<api::MethodCallSuccess, Error> {
    w::wasm_bindgen_futures::spawn_local(async move {
        let result =
            subscriber_background_future(env, server.clone(), common_args, args, subscription_id)
                .await;
        // TODO actual handling
        match result {
            Ok(_) => {
                console_log!("A websocket ended")
            }
            Err(_) => {
                server.nfsendj(&api::ServerToClientMessage::Info("Closed :(".to_string()));
            }
        }
    });

    Ok(api::MethodCallSuccess::Ack)
}

pub async fn unsubscribe_from_room() -> Result<api::MethodCallSuccess, Error> {
    todo!();
}

pub async fn add_privileged_peer(
    env: &w::Env,
    common_args: api::MethodCallCommonArgs,
    args: api::AddPrivilegedPeerArgs,
) -> Result<api::MethodCallSuccess, Error> {
    let room_id = args.room_id;
    let request = room_api::AddPrivilegedPeerMessage {
        adder_id: common_args.ecdsa_public_key,
        added_id: args.allow_ecdsa_public_key,
    }
    .into_request()?;
    let stub = get_room_stub(env, room_id)?;
    // Make sure that the room returns a boolean to determine that it didn't fail in an unexpected way,
    // but don't care about the actual result to hide info from clients
    let _ = serde_json::from_str::<bool>(&stub.fetch_with_request(request).await?.text().await?);
    Ok(api::MethodCallSuccess::Ack)
}

pub async fn get_room_data_history() -> Result<api::MethodCallSuccess, Error> {
    todo!();
}
pub async fn delete_data() -> Result<api::MethodCallSuccess, Error> {
    todo!();
}

pub async fn broadcast_data(
    env: &w::Env,
    common_args: api::MethodCallCommonArgs,
    args: api::BroadcastDataArgs,
) -> Result<api::MethodCallSuccess, Error> {
    let args = args.common_args;
    let request = room_api::BroadcastDataMessage {
        data: args.data,
        sender_id: common_args.ecdsa_public_key,
        nonce: common_args.nonce,
        write_history: args.write_history,
    }
    .into_request()?;
    let stub = get_room_stub(env, args.room_id)?;
    let _ = serde_json::from_str::<bool>(&stub.fetch_with_request(request).await?.text().await?);
    Ok(api::MethodCallSuccess::Ack)
}

pub async fn unicast_data() -> Result<api::MethodCallSuccess, Error> {
    todo!();
}
