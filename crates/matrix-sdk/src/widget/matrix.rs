// Copyright 2023 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Matrix driver implementation that exposes Matrix functionality
//! that is relevant for the widget API.

use std::collections::BTreeMap;

use matrix_sdk_base::deserialized_responses::RawAnySyncOrStrippedState;
use ruma::{
    api::client::{
        account::request_openid_token::v3::{Request as OpenIdRequest, Response as OpenIdResponse},
        delayed_events::{self, update_delayed_event::unstable::UpdateAction},
        filter::RoomEventFilter,
    },
    assign,
    events::{
        AnyMessageLikeEventContent, AnyStateEventContent, AnySyncMessageLikeEvent,
        AnySyncStateEvent, AnySyncTimelineEvent, AnyTimelineEvent, MessageLikeEventType,
        StateEventType, TimelineEventType,
    },
    serde::{from_raw_json_value, Raw},
    EventId, RoomId, TransactionId,
};
use serde_json::{value::RawValue as RawJsonValue, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tracing::error;

use super::{machine::SendEventResponse, StateKeySelector};
use crate::{event_handler::EventHandlerDropGuard, room::MessagesOptions, Error, Result, Room};

/// Thin wrapper around a [`Room`] that provides functionality relevant for
/// widgets.
pub(crate) struct MatrixDriver {
    room: Room,
}

impl MatrixDriver {
    /// Creates a new `MatrixDriver` for a given `room`.
    pub(crate) fn new(room: Room) -> Self {
        Self { room }
    }

    /// Requests an OpenID token for the current user.
    pub(crate) async fn get_open_id(&self) -> Result<OpenIdResponse> {
        let user_id = self.room.own_user_id().to_owned();
        self.room
            .client
            .send(OpenIdRequest::new(user_id))
            .await
            .map_err(|error| Error::Http(Box::new(error)))
    }

    /// Reads the latest `limit` events of a given `event_type` from the room.
    pub(crate) async fn read_message_like_events(
        &self,
        event_type: MessageLikeEventType,
        limit: u32,
    ) -> Result<Vec<Raw<AnyTimelineEvent>>> {
        let options = assign!(MessagesOptions::backward(), {
            limit: limit.into(),
            filter: assign!(RoomEventFilter::default(), {
                types: Some(vec![event_type.to_string()])
            }),
        });

        let messages = self.room.messages(options).await?;
        Ok(messages.chunk.into_iter().map(|ev| ev.into_raw().cast()).collect())
    }

    pub(crate) async fn read_state_events(
        &self,
        event_type: StateEventType,
        state_key: &StateKeySelector,
    ) -> Result<Vec<Raw<AnyTimelineEvent>>> {
        let room_id = self.room.room_id();
        let convert = |sync_or_stripped_state| match sync_or_stripped_state {
            RawAnySyncOrStrippedState::Sync(ev) => Some(attach_room_id(ev.cast_ref(), room_id)),
            RawAnySyncOrStrippedState::Stripped(_) => {
                error!("MatrixDriver can't operate in invited rooms");
                None
            }
        };

        let events = match state_key {
            StateKeySelector::Key(state_key) => self
                .room
                .get_state_event(event_type, state_key)
                .await?
                .and_then(convert)
                .into_iter()
                .collect(),
            StateKeySelector::Any => {
                let events = self.room.get_state_events(event_type).await?;
                events.into_iter().filter_map(convert).collect()
            }
        };

        Ok(events)
    }

    /// Sends the given `event` to the room.
    ///
    /// This method allows the widget machine to handle widget requests by
    /// providing a unified, high-level widget-specific API for sending events
    /// to the room.
    pub(crate) async fn send(
        &self,
        event_type: TimelineEventType,
        state_key: Option<String>,
        content: Box<RawJsonValue>,
        delayed_event_parameters: Option<delayed_events::DelayParameters>,
    ) -> Result<SendEventResponse> {
        let type_str = event_type.to_string();

        if let Some(redacts) = from_raw_json_value::<Value, serde_json::Error>(&content)
            .ok()
            .and_then(|b| b["redacts"].as_str().and_then(|s| EventId::parse(s).ok()))
        {
            return Ok(SendEventResponse::from_event_id(
                self.room.redact(&redacts, None, None).await?.event_id,
            ));
        }

        Ok(match (state_key, delayed_event_parameters) {
            (None, None) => SendEventResponse::from_event_id(
                self.room.send_raw(&type_str, content).await?.event_id,
            ),

            (Some(key), None) => SendEventResponse::from_event_id(
                self.room.send_state_event_raw(&type_str, &key, content).await?.event_id,
            ),

            (None, Some(delayed_event_parameters)) => {
                let r = delayed_events::delayed_message_event::unstable::Request::new_raw(
                    self.room.room_id().to_owned(),
                    TransactionId::new(),
                    MessageLikeEventType::from(type_str),
                    delayed_event_parameters,
                    Raw::<AnyMessageLikeEventContent>::from_json(content),
                );
                self.room.client.send(r).await.map(|r| r.into())?
            }

            (Some(key), Some(delayed_event_parameters)) => {
                let r = delayed_events::delayed_state_event::unstable::Request::new_raw(
                    self.room.room_id().to_owned(),
                    key,
                    StateEventType::from(type_str),
                    delayed_event_parameters,
                    Raw::<AnyStateEventContent>::from_json(content),
                );
                self.room.client.send(r).await.map(|r| r.into())?
            }
        })
    }

    /// Send a request to the `/delayed_events`` endpoint ([MSC4140](https://github.com/matrix-org/matrix-spec-proposals/pull/4140))
    /// This can be used to refresh cancel or send a Delayed Event (An Event
    /// that is send ahead of time to the homeserver and gets distributed
    /// once it times out.)
    pub(crate) async fn update_delayed_event(
        &self,
        delay_id: String,
        action: UpdateAction,
    ) -> Result<delayed_events::update_delayed_event::unstable::Response> {
        let r = delayed_events::update_delayed_event::unstable::Request::new(delay_id, action);
        self.room.client.send(r).await.map_err(|error| Error::Http(Box::new(error)))
    }

    /// Starts forwarding new room events. Once the returned `EventReceiver`
    /// is dropped, forwarding will be stopped.
    pub(crate) fn events(&self) -> EventReceiver {
        let (tx, rx) = unbounded_channel();
        let room_id = self.room.room_id().to_owned();

        // Get only message like events from the timeline section of the sync.
        let _tx = tx.clone();
        let _room_id = room_id.clone();
        let handle_msg_like =
            self.room.add_event_handler(move |raw: Raw<AnySyncMessageLikeEvent>| {
                let _ = _tx.send(attach_room_id(raw.cast_ref(), &_room_id));
                async {}
            });
        let drop_guard_msg_like = self.room.client().event_handler_drop_guard(handle_msg_like);

        // Get only all state events from the state section of the sync.
        let handle_state = self.room.add_event_handler(move |raw: Raw<AnySyncStateEvent>| {
            let _ = tx.send(attach_room_id(raw.cast_ref(), &room_id));
            async {}
        });
        let drop_guard_state = self.room.client().event_handler_drop_guard(handle_state);

        // The receiver will get a combination of state and message like events.
        // The state events will come from the state section of the sync (to always
        // represent current resolved state). All state events in the timeline
        // section of the sync will not be forwarded to the widget.
        // TODO annotate the events and send both timeline and state section state
        // events.
        EventReceiver { rx, _drop_guards: [drop_guard_msg_like, drop_guard_state] }
    }
}

/// A simple entity that wraps an `UnboundedReceiver`
/// along with the drop guard for the room event handler.
pub(crate) struct EventReceiver {
    rx: UnboundedReceiver<Raw<AnyTimelineEvent>>,
    _drop_guards: [EventHandlerDropGuard; 2],
}

impl EventReceiver {
    pub(crate) async fn recv(&mut self) -> Option<Raw<AnyTimelineEvent>> {
        self.rx.recv().await
    }
}

fn attach_room_id(raw_ev: &Raw<AnySyncTimelineEvent>, room_id: &RoomId) -> Raw<AnyTimelineEvent> {
    let mut ev_obj = raw_ev.deserialize_as::<BTreeMap<String, Box<RawJsonValue>>>().unwrap();
    ev_obj.insert("room_id".to_owned(), serde_json::value::to_raw_value(room_id).unwrap());
    Raw::new(&ev_obj).unwrap().cast()
}
