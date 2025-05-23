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

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use extension_trait::extension_trait;
use matrix_sdk::attachment::{BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo};
use ruma::{
    assign,
    events::{
        call::notify::NotifyType as RumaNotifyType,
        location::AssetType as RumaAssetType,
        poll::start::PollKind as RumaPollKind,
        room::{
            message::{
                AudioInfo as RumaAudioInfo,
                AudioMessageEventContent as RumaAudioMessageEventContent,
                EmoteMessageEventContent as RumaEmoteMessageEventContent, FileInfo as RumaFileInfo,
                FileMessageEventContent as RumaFileMessageEventContent,
                FormattedBody as RumaFormattedBody,
                ImageMessageEventContent as RumaImageMessageEventContent,
                LocationMessageEventContent as RumaLocationMessageEventContent,
                MessageType as RumaMessageType,
                NoticeMessageEventContent as RumaNoticeMessageEventContent,
                RoomMessageEventContentWithoutRelation,
                TextMessageEventContent as RumaTextMessageEventContent, UnstableAmplitude,
                UnstableAudioDetailsContentBlock as RumaUnstableAudioDetailsContentBlock,
                UnstableVoiceContentBlock as RumaUnstableVoiceContentBlock,
                VideoInfo as RumaVideoInfo,
                VideoMessageEventContent as RumaVideoMessageEventContent,
            },
            ImageInfo as RumaImageInfo, MediaSource as RumaMediaSource,
            ThumbnailInfo as RumaThumbnailInfo,
        },
    },
    matrix_uri::MatrixId as RumaMatrixId,
    serde::JsonObject,
    MatrixToUri, MatrixUri as RumaMatrixUri, OwnedUserId, UInt, UserId,
};
use tracing::info;

use crate::{
    error::{ClientError, MediaInfoError},
    helpers::unwrap_or_clone_arc,
    timeline::MessageContent,
    utils::u64_to_uint,
};

#[derive(uniffi::Enum)]
pub enum AuthData {
    /// Password-based authentication (`m.login.password`).
    Password { password_details: AuthDataPasswordDetails },
}

#[derive(uniffi::Record)]
pub struct AuthDataPasswordDetails {
    /// One of the user's identifiers.
    identifier: String,

    /// The plaintext password.
    password: String,
}

impl From<AuthData> for ruma::api::client::uiaa::AuthData {
    fn from(value: AuthData) -> ruma::api::client::uiaa::AuthData {
        match value {
            AuthData::Password { password_details } => {
                let user_id = ruma::UserId::parse(password_details.identifier).unwrap();

                ruma::api::client::uiaa::AuthData::Password(ruma::api::client::uiaa::Password::new(
                    user_id.into(),
                    password_details.password,
                ))
            }
        }
    }
}

/// Parse a matrix entity from a given URI, be it either
/// a `matrix.to` link or a `matrix:` URI
#[matrix_sdk_ffi_macros::export]
pub fn parse_matrix_entity_from(uri: String) -> Option<MatrixEntity> {
    if let Ok(matrix_uri) = RumaMatrixUri::parse(&uri) {
        return Some(MatrixEntity {
            id: matrix_uri.id().into(),
            via: matrix_uri.via().iter().map(|via| via.to_string()).collect(),
        });
    }

    if let Ok(matrix_to_uri) = MatrixToUri::parse(&uri) {
        return Some(MatrixEntity {
            id: matrix_to_uri.id().into(),
            via: matrix_to_uri.via().iter().map(|via| via.to_string()).collect(),
        });
    }

    None
}

/// A Matrix entity that can be a room, room alias, user, or event, and a list
/// of via servers.
#[derive(uniffi::Record)]
pub struct MatrixEntity {
    id: MatrixId,
    via: Vec<String>,
}

/// A Matrix ID that can be a room, room alias, user, or event.
#[derive(Clone, uniffi::Enum)]
pub enum MatrixId {
    Room { id: String },
    RoomAlias { alias: String },
    User { id: String },
    EventOnRoomId { room_id: String, event_id: String },
    EventOnRoomAlias { alias: String, event_id: String },
}

impl From<&RumaMatrixId> for MatrixId {
    fn from(value: &RumaMatrixId) -> Self {
        match value {
            RumaMatrixId::User(id) => MatrixId::User { id: id.to_string() },
            RumaMatrixId::Room(id) => MatrixId::Room { id: id.to_string() },
            RumaMatrixId::RoomAlias(id) => MatrixId::RoomAlias { alias: id.to_string() },

            RumaMatrixId::Event(room_id_or_alias, event_id) => {
                if room_id_or_alias.is_room_id() {
                    MatrixId::EventOnRoomId {
                        room_id: room_id_or_alias.to_string(),
                        event_id: event_id.to_string(),
                    }
                } else if room_id_or_alias.is_room_alias_id() {
                    MatrixId::EventOnRoomAlias {
                        alias: room_id_or_alias.to_string(),
                        event_id: event_id.to_string(),
                    }
                } else {
                    panic!("Unexpected MatrixId type: {:?}", room_id_or_alias)
                }
            }
            _ => panic!("Unexpected MatrixId type: {:?}", value),
        }
    }
}

#[matrix_sdk_ffi_macros::export]
pub fn message_event_content_new(
    msgtype: MessageType,
) -> Result<Arc<RoomMessageEventContentWithoutRelation>, ClientError> {
    Ok(Arc::new(RoomMessageEventContentWithoutRelation::new(msgtype.try_into()?)))
}

#[matrix_sdk_ffi_macros::export]
pub fn message_event_content_from_markdown(
    md: String,
) -> Arc<RoomMessageEventContentWithoutRelation> {
    Arc::new(RoomMessageEventContentWithoutRelation::new(RumaMessageType::text_markdown(md)))
}

#[matrix_sdk_ffi_macros::export]
pub fn message_event_content_from_markdown_as_emote(
    md: String,
) -> Arc<RoomMessageEventContentWithoutRelation> {
    Arc::new(RoomMessageEventContentWithoutRelation::new(RumaMessageType::emote_markdown(md)))
}

#[matrix_sdk_ffi_macros::export]
pub fn message_event_content_from_html(
    body: String,
    html_body: String,
) -> Arc<RoomMessageEventContentWithoutRelation> {
    Arc::new(RoomMessageEventContentWithoutRelation::new(RumaMessageType::text_html(
        body, html_body,
    )))
}

#[matrix_sdk_ffi_macros::export]
pub fn message_event_content_from_html_as_emote(
    body: String,
    html_body: String,
) -> Arc<RoomMessageEventContentWithoutRelation> {
    Arc::new(RoomMessageEventContentWithoutRelation::new(RumaMessageType::emote_html(
        body, html_body,
    )))
}

#[derive(Clone, uniffi::Object)]
pub struct MediaSource {
    pub(crate) media_source: RumaMediaSource,
}

#[matrix_sdk_ffi_macros::export]
impl MediaSource {
    #[uniffi::constructor]
    pub fn from_url(url: String) -> Result<Arc<MediaSource>, ClientError> {
        let media_source = RumaMediaSource::Plain(url.into());
        media_source.verify()?;

        Ok(Arc::new(MediaSource { media_source }))
    }

    pub fn url(&self) -> String {
        self.media_source.url()
    }

    // Used on Element X Android
    #[uniffi::constructor]
    pub fn from_json(json: String) -> Result<Arc<Self>, ClientError> {
        let media_source: RumaMediaSource = serde_json::from_str(&json)?;
        media_source.verify()?;

        Ok(Arc::new(MediaSource { media_source }))
    }

    // Used on Element X Android
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.media_source)
            .expect("Media source should always be serializable ")
    }
}

impl TryFrom<RumaMediaSource> for MediaSource {
    type Error = ClientError;

    fn try_from(value: RumaMediaSource) -> Result<Self, Self::Error> {
        value.verify()?;
        Ok(Self { media_source: value })
    }
}

impl TryFrom<&RumaMediaSource> for MediaSource {
    type Error = ClientError;

    fn try_from(value: &RumaMediaSource) -> Result<Self, Self::Error> {
        value.verify()?;
        Ok(Self { media_source: value.clone() })
    }
}

impl From<MediaSource> for RumaMediaSource {
    fn from(value: MediaSource) -> Self {
        value.media_source
    }
}

#[extension_trait]
pub(crate) impl MediaSourceExt for RumaMediaSource {
    fn verify(&self) -> Result<(), ClientError> {
        match self {
            RumaMediaSource::Plain(url) => {
                url.validate().map_err(|e| ClientError::Generic { msg: e.to_string() })?;
            }
            RumaMediaSource::Encrypted(file) => {
                file.url.validate().map_err(|e| ClientError::Generic { msg: e.to_string() })?;
            }
        }

        Ok(())
    }

    fn url(&self) -> String {
        match self {
            RumaMediaSource::Plain(url) => url.to_string(),
            RumaMediaSource::Encrypted(file) => file.url.to_string(),
        }
    }
}

#[extension_trait]
pub impl RoomMessageEventContentWithoutRelationExt for RoomMessageEventContentWithoutRelation {
    fn with_mentions(self: Arc<Self>, mentions: Mentions) -> Arc<Self> {
        let mut content = unwrap_or_clone_arc(self);
        content.mentions = Some(mentions.into());
        Arc::new(content)
    }
}

#[derive(Clone)]
pub struct Mentions {
    pub user_ids: Vec<String>,
    pub room: bool,
}

impl From<Mentions> for ruma::events::Mentions {
    fn from(value: Mentions) -> Self {
        let mut user_ids = BTreeSet::<OwnedUserId>::new();
        for user_id in value.user_ids {
            if let Ok(user_id) = UserId::parse(user_id) {
                user_ids.insert(user_id);
            }
        }
        let mut result = Self::default();
        result.user_ids = user_ids;
        result.room = value.room;
        result
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum MessageType {
    Emote { content: EmoteMessageContent },
    Image { content: ImageMessageContent },
    Audio { content: AudioMessageContent },
    Video { content: VideoMessageContent },
    File { content: FileMessageContent },
    Notice { content: NoticeMessageContent },
    Text { content: TextMessageContent },
    Location { content: LocationContent },
    Other { msgtype: String, body: String },
}

/// From MSC2530: https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/2530-body-as-caption.md
/// If the filename field is present in a media message, clients should treat
/// body as a caption instead of a file name. Otherwise, the body is the
/// file name.
///
/// So:
/// - if a media has a filename and a caption, the body is the caption, filename
///   is its own field.
/// - if a media only has a filename, then body is the filename.
fn get_body_and_filename(filename: String, caption: Option<String>) -> (String, Option<String>) {
    if let Some(caption) = caption {
        (caption, Some(filename))
    } else {
        (filename, None)
    }
}

impl TryFrom<MessageType> for RumaMessageType {
    type Error = ClientError;

    fn try_from(value: MessageType) -> Result<Self, Self::Error> {
        Ok(match value {
            MessageType::Emote { content } => {
                Self::Emote(assign!(RumaEmoteMessageEventContent::plain(content.body), {
                    formatted: content.formatted.map(Into::into),
                }))
            }
            MessageType::Image { content } => {
                let (body, filename) = get_body_and_filename(content.filename, content.caption);
                let mut event_content =
                    RumaImageMessageEventContent::new(body, (*content.source).clone().into())
                        .info(content.info.map(Into::into).map(Box::new));
                event_content.formatted = content.formatted_caption.map(Into::into);
                event_content.filename = filename;
                Self::Image(event_content)
            }
            MessageType::Audio { content } => {
                let (body, filename) = get_body_and_filename(content.filename, content.caption);
                let mut event_content =
                    RumaAudioMessageEventContent::new(body, (*content.source).clone().into())
                        .info(content.info.map(Into::into).map(Box::new));
                event_content.formatted = content.formatted_caption.map(Into::into);
                event_content.filename = filename;
                event_content.audio = content.audio.map(Into::into);
                event_content.voice = content.voice.map(Into::into);
                Self::Audio(event_content)
            }
            MessageType::Video { content } => {
                let (body, filename) = get_body_and_filename(content.filename, content.caption);
                let mut event_content =
                    RumaVideoMessageEventContent::new(body, (*content.source).clone().into())
                        .info(content.info.map(Into::into).map(Box::new));
                event_content.formatted = content.formatted_caption.map(Into::into);
                event_content.filename = filename;
                Self::Video(event_content)
            }
            MessageType::File { content } => {
                let (body, filename) = get_body_and_filename(content.filename, content.caption);
                let mut event_content =
                    RumaFileMessageEventContent::new(body, (*content.source).clone().into())
                        .info(content.info.map(Into::into).map(Box::new));
                event_content.formatted = content.formatted_caption.map(Into::into);
                event_content.filename = filename;
                Self::File(event_content)
            }
            MessageType::Notice { content } => {
                Self::Notice(assign!(RumaNoticeMessageEventContent::plain(content.body), {
                    formatted: content.formatted.map(Into::into),
                }))
            }
            MessageType::Text { content } => {
                Self::Text(assign!(RumaTextMessageEventContent::plain(content.body), {
                    formatted: content.formatted.map(Into::into),
                }))
            }
            MessageType::Location { content } => {
                Self::Location(RumaLocationMessageEventContent::new(content.body, content.geo_uri))
            }
            MessageType::Other { msgtype, body } => {
                Self::new(&msgtype, body, JsonObject::default())?
            }
        })
    }
}

impl TryFrom<RumaMessageType> for MessageType {
    type Error = ClientError;

    fn try_from(value: RumaMessageType) -> Result<Self, Self::Error> {
        Ok(match value {
            RumaMessageType::Emote(c) => MessageType::Emote {
                content: EmoteMessageContent {
                    body: c.body.clone(),
                    formatted: c.formatted.as_ref().map(Into::into),
                },
            },
            RumaMessageType::Image(c) => MessageType::Image {
                content: ImageMessageContent {
                    filename: c.filename().to_owned(),
                    caption: c.caption().map(ToString::to_string),
                    formatted_caption: c.formatted_caption().map(Into::into),
                    source: Arc::new(c.source.try_into()?),
                    info: c.info.as_deref().map(TryInto::try_into).transpose()?,
                },
            },

            RumaMessageType::Audio(c) => MessageType::Audio {
                content: AudioMessageContent {
                    filename: c.filename().to_owned(),
                    caption: c.caption().map(ToString::to_string),
                    formatted_caption: c.formatted_caption().map(Into::into),
                    source: Arc::new(c.source.try_into()?),
                    info: c.info.as_deref().map(Into::into),
                    audio: c.audio.map(Into::into),
                    voice: c.voice.map(Into::into),
                },
            },
            RumaMessageType::Video(c) => MessageType::Video {
                content: VideoMessageContent {
                    filename: c.filename().to_owned(),
                    caption: c.caption().map(ToString::to_string),
                    formatted_caption: c.formatted_caption().map(Into::into),
                    source: Arc::new(c.source.try_into()?),
                    info: c.info.as_deref().map(TryInto::try_into).transpose()?,
                },
            },
            RumaMessageType::File(c) => MessageType::File {
                content: FileMessageContent {
                    filename: c.filename().to_owned(),
                    caption: c.caption().map(ToString::to_string),
                    formatted_caption: c.formatted_caption().map(Into::into),
                    source: Arc::new(c.source.try_into()?),
                    info: c.info.as_deref().map(TryInto::try_into).transpose()?,
                },
            },
            RumaMessageType::Notice(c) => MessageType::Notice {
                content: NoticeMessageContent {
                    body: c.body.clone(),
                    formatted: c.formatted.as_ref().map(Into::into),
                },
            },
            RumaMessageType::Text(c) => MessageType::Text {
                content: TextMessageContent {
                    body: c.body.clone(),
                    formatted: c.formatted.as_ref().map(Into::into),
                },
            },
            RumaMessageType::Location(c) => {
                let (description, zoom_level) =
                    c.location.map(|loc| (loc.description, loc.zoom_level)).unwrap_or((None, None));
                MessageType::Location {
                    content: LocationContent {
                        body: c.body,
                        geo_uri: c.geo_uri,
                        description,
                        zoom_level: zoom_level.and_then(|z| z.get().try_into().ok()),
                        asset: c.asset.and_then(|a| match a.type_ {
                            RumaAssetType::Self_ => Some(AssetType::Sender),
                            RumaAssetType::Pin => Some(AssetType::Pin),
                            _ => None,
                        }),
                    },
                }
            }
            _ => MessageType::Other {
                msgtype: value.msgtype().to_owned(),
                body: value.body().to_owned(),
            },
        })
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum NotifyType {
    Ring,
    Notify,
}

impl From<RumaNotifyType> for NotifyType {
    fn from(val: RumaNotifyType) -> Self {
        match val {
            RumaNotifyType::Ring => Self::Ring,
            _ => Self::Notify,
        }
    }
}

impl From<NotifyType> for RumaNotifyType {
    fn from(value: NotifyType) -> Self {
        match value {
            NotifyType::Ring => RumaNotifyType::Ring,
            NotifyType::Notify => RumaNotifyType::Notify,
        }
    }
}

#[derive(Clone, uniffi::Record)]
pub struct EmoteMessageContent {
    pub body: String,
    pub formatted: Option<FormattedBody>,
}

#[derive(Clone, uniffi::Record)]
pub struct ImageMessageContent {
    /// The computed filename, for use in a client.
    pub filename: String,
    pub caption: Option<String>,
    pub formatted_caption: Option<FormattedBody>,
    pub source: Arc<MediaSource>,
    pub info: Option<ImageInfo>,
}

#[derive(Clone, uniffi::Record)]
pub struct AudioMessageContent {
    /// The computed filename, for use in a client.
    pub filename: String,
    pub caption: Option<String>,
    pub formatted_caption: Option<FormattedBody>,
    pub source: Arc<MediaSource>,
    pub info: Option<AudioInfo>,
    pub audio: Option<UnstableAudioDetailsContent>,
    pub voice: Option<UnstableVoiceContent>,
}

#[derive(Clone, uniffi::Record)]
pub struct VideoMessageContent {
    /// The computed filename, for use in a client.
    pub filename: String,
    pub caption: Option<String>,
    pub formatted_caption: Option<FormattedBody>,
    pub source: Arc<MediaSource>,
    pub info: Option<VideoInfo>,
}

#[derive(Clone, uniffi::Record)]
pub struct FileMessageContent {
    /// The computed filename, for use in a client.
    pub filename: String,
    pub caption: Option<String>,
    pub formatted_caption: Option<FormattedBody>,
    pub source: Arc<MediaSource>,
    pub info: Option<FileInfo>,
}

#[derive(Clone, uniffi::Record)]
pub struct ImageInfo {
    pub height: Option<u64>,
    pub width: Option<u64>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub thumbnail_info: Option<ThumbnailInfo>,
    pub thumbnail_source: Option<Arc<MediaSource>>,
    pub blurhash: Option<String>,
    pub is_animated: Option<bool>,
}

impl From<ImageInfo> for RumaImageInfo {
    fn from(value: ImageInfo) -> Self {
        assign!(RumaImageInfo::new(), {
            height: value.height.map(u64_to_uint),
            width: value.width.map(u64_to_uint),
            mimetype: value.mimetype,
            size: value.size.map(u64_to_uint),
            thumbnail_info: value.thumbnail_info.map(Into::into).map(Box::new),
            thumbnail_source: value.thumbnail_source.map(|source| (*source).clone().into()),
            blurhash: value.blurhash,
            is_animated: value.is_animated,
        })
    }
}

impl TryFrom<&ImageInfo> for BaseImageInfo {
    type Error = MediaInfoError;

    fn try_from(value: &ImageInfo) -> Result<Self, MediaInfoError> {
        let height = UInt::try_from(value.height.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let width = UInt::try_from(value.width.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let size = UInt::try_from(value.size.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let blurhash = value.blurhash.clone().ok_or(MediaInfoError::MissingField)?;

        Ok(BaseImageInfo {
            height: Some(height),
            width: Some(width),
            size: Some(size),
            blurhash: Some(blurhash),
            is_animated: value.is_animated,
        })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct AudioInfo {
    pub duration: Option<Duration>,
    pub size: Option<u64>,
    pub mimetype: Option<String>,
}

impl From<AudioInfo> for RumaAudioInfo {
    fn from(value: AudioInfo) -> Self {
        assign!(RumaAudioInfo::new(), {
            duration: value.duration,
            size: value.size.map(u64_to_uint),
            mimetype: value.mimetype,
        })
    }
}

impl TryFrom<&AudioInfo> for BaseAudioInfo {
    type Error = MediaInfoError;

    fn try_from(value: &AudioInfo) -> Result<Self, MediaInfoError> {
        let duration = value.duration.ok_or(MediaInfoError::MissingField)?;
        let size = UInt::try_from(value.size.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;

        Ok(BaseAudioInfo { duration: Some(duration), size: Some(size) })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct UnstableAudioDetailsContent {
    pub duration: Duration,
    pub waveform: Vec<u16>,
}

impl From<RumaUnstableAudioDetailsContentBlock> for UnstableAudioDetailsContent {
    fn from(details: RumaUnstableAudioDetailsContentBlock) -> Self {
        Self {
            duration: details.duration,
            waveform: details
                .waveform
                .iter()
                .map(|x| u16::try_from(x.get()).unwrap_or(0))
                .collect(),
        }
    }
}

impl From<UnstableAudioDetailsContent> for RumaUnstableAudioDetailsContentBlock {
    fn from(details: UnstableAudioDetailsContent) -> Self {
        Self::new(
            details.duration,
            details.waveform.iter().map(|x| UnstableAmplitude::new(x.to_owned())).collect(),
        )
    }
}

#[derive(Clone, uniffi::Record)]
pub struct UnstableVoiceContent {}

impl From<RumaUnstableVoiceContentBlock> for UnstableVoiceContent {
    fn from(_details: RumaUnstableVoiceContentBlock) -> Self {
        Self {}
    }
}

impl From<UnstableVoiceContent> for RumaUnstableVoiceContentBlock {
    fn from(_details: UnstableVoiceContent) -> Self {
        Self::new()
    }
}

#[derive(Clone, uniffi::Record)]
pub struct VideoInfo {
    pub duration: Option<Duration>,
    pub height: Option<u64>,
    pub width: Option<u64>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub thumbnail_info: Option<ThumbnailInfo>,
    pub thumbnail_source: Option<Arc<MediaSource>>,
    pub blurhash: Option<String>,
}

impl From<VideoInfo> for RumaVideoInfo {
    fn from(value: VideoInfo) -> Self {
        assign!(RumaVideoInfo::new(), {
            duration: value.duration,
            height: value.height.map(u64_to_uint),
            width: value.width.map(u64_to_uint),
            mimetype: value.mimetype,
            size: value.size.map(u64_to_uint),
            thumbnail_info: value.thumbnail_info.map(Into::into).map(Box::new),
            thumbnail_source: value.thumbnail_source.map(|source| (*source).clone().into()),
            blurhash: value.blurhash,
        })
    }
}

impl TryFrom<&VideoInfo> for BaseVideoInfo {
    type Error = MediaInfoError;

    fn try_from(value: &VideoInfo) -> Result<Self, MediaInfoError> {
        let duration = value.duration.ok_or(MediaInfoError::MissingField)?;
        let height = UInt::try_from(value.height.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let width = UInt::try_from(value.width.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let size = UInt::try_from(value.size.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;
        let blurhash = value.blurhash.clone().ok_or(MediaInfoError::MissingField)?;

        Ok(BaseVideoInfo {
            duration: Some(duration),
            height: Some(height),
            width: Some(width),
            size: Some(size),
            blurhash: Some(blurhash),
        })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct FileInfo {
    pub mimetype: Option<String>,
    pub size: Option<u64>,
    pub thumbnail_info: Option<ThumbnailInfo>,
    pub thumbnail_source: Option<Arc<MediaSource>>,
}

impl From<FileInfo> for RumaFileInfo {
    fn from(value: FileInfo) -> Self {
        assign!(RumaFileInfo::new(), {
            mimetype: value.mimetype,
            size: value.size.map(u64_to_uint),
            thumbnail_info: value.thumbnail_info.map(Into::into).map(Box::new),
            thumbnail_source: value.thumbnail_source.map(|source| (*source).clone().into()),
        })
    }
}

impl TryFrom<&FileInfo> for BaseFileInfo {
    type Error = MediaInfoError;

    fn try_from(value: &FileInfo) -> Result<Self, MediaInfoError> {
        let size = UInt::try_from(value.size.ok_or(MediaInfoError::MissingField)?)
            .map_err(|_| MediaInfoError::InvalidField)?;

        Ok(BaseFileInfo { size: Some(size) })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct ThumbnailInfo {
    pub height: Option<u64>,
    pub width: Option<u64>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,
}

impl From<ThumbnailInfo> for RumaThumbnailInfo {
    fn from(value: ThumbnailInfo) -> Self {
        assign!(RumaThumbnailInfo::new(), {
            height: value.height.map(u64_to_uint),
            width: value.width.map(u64_to_uint),
            mimetype: value.mimetype,
            size: value.size.map(u64_to_uint),
        })
    }
}

#[derive(Clone, uniffi::Record)]
pub struct NoticeMessageContent {
    pub body: String,
    pub formatted: Option<FormattedBody>,
}

#[derive(Clone, uniffi::Record)]
pub struct TextMessageContent {
    pub body: String,
    pub formatted: Option<FormattedBody>,
}

#[derive(Clone, uniffi::Record)]
pub struct LocationContent {
    pub body: String,
    pub geo_uri: String,
    pub description: Option<String>,
    pub zoom_level: Option<u8>,
    pub asset: Option<AssetType>,
}

#[derive(Clone, uniffi::Enum)]
pub enum AssetType {
    Sender,
    Pin,
}

impl From<AssetType> for RumaAssetType {
    fn from(value: AssetType) -> Self {
        match value {
            AssetType::Sender => Self::Self_,
            AssetType::Pin => Self::Pin,
        }
    }
}

#[derive(Clone, uniffi::Record)]
pub struct FormattedBody {
    pub format: MessageFormat,
    pub body: String,
}

impl From<FormattedBody> for RumaFormattedBody {
    fn from(f: FormattedBody) -> Self {
        Self {
            format: match f.format {
                MessageFormat::Html => matrix_sdk::ruma::events::room::message::MessageFormat::Html,
                MessageFormat::Unknown { format } => format.into(),
            },
            body: f.body,
        }
    }
}

impl From<&RumaFormattedBody> for FormattedBody {
    fn from(f: &RumaFormattedBody) -> Self {
        Self {
            format: match &f.format {
                matrix_sdk::ruma::events::room::message::MessageFormat::Html => MessageFormat::Html,
                _ => MessageFormat::Unknown { format: f.format.to_string() },
            },
            body: f.body.clone(),
        }
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum MessageFormat {
    Html,
    Unknown { format: String },
}

impl TryFrom<&matrix_sdk::ruma::events::room::ImageInfo> for ImageInfo {
    type Error = ClientError;

    fn try_from(info: &matrix_sdk::ruma::events::room::ImageInfo) -> Result<Self, Self::Error> {
        let thumbnail_info = info.thumbnail_info.as_ref().map(|info| ThumbnailInfo {
            height: info.height.map(Into::into),
            width: info.width.map(Into::into),
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
        });

        Ok(Self {
            height: info.height.map(Into::into),
            width: info.width.map(Into::into),
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
            thumbnail_info,
            thumbnail_source: info
                .thumbnail_source
                .as_ref()
                .map(TryInto::try_into)
                .transpose()?
                .map(Arc::new),
            blurhash: info.blurhash.clone(),
            is_animated: info.is_animated,
        })
    }
}

impl From<&RumaAudioInfo> for AudioInfo {
    fn from(info: &RumaAudioInfo) -> Self {
        Self {
            duration: info.duration,
            size: info.size.map(Into::into),
            mimetype: info.mimetype.clone(),
        }
    }
}

impl TryFrom<&RumaVideoInfo> for VideoInfo {
    type Error = ClientError;

    fn try_from(info: &RumaVideoInfo) -> Result<Self, Self::Error> {
        let thumbnail_info = info.thumbnail_info.as_ref().map(|info| ThumbnailInfo {
            height: info.height.map(Into::into),
            width: info.width.map(Into::into),
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
        });

        Ok(Self {
            duration: info.duration,
            height: info.height.map(Into::into),
            width: info.width.map(Into::into),
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
            thumbnail_info,
            thumbnail_source: info
                .thumbnail_source
                .as_ref()
                .map(TryInto::try_into)
                .transpose()?
                .map(Arc::new),
            blurhash: info.blurhash.clone(),
        })
    }
}

impl TryFrom<&RumaFileInfo> for FileInfo {
    type Error = ClientError;

    fn try_from(info: &RumaFileInfo) -> Result<Self, Self::Error> {
        let thumbnail_info = info.thumbnail_info.as_ref().map(|info| ThumbnailInfo {
            height: info.height.map(Into::into),
            width: info.width.map(Into::into),
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
        });

        Ok(Self {
            mimetype: info.mimetype.clone(),
            size: info.size.map(Into::into),
            thumbnail_info,
            thumbnail_source: info
                .thumbnail_source
                .as_ref()
                .map(TryInto::try_into)
                .transpose()?
                .map(Arc::new),
        })
    }
}

#[derive(Clone, uniffi::Enum)]
pub enum PollKind {
    Disclosed,
    Undisclosed,
}

impl From<PollKind> for RumaPollKind {
    fn from(value: PollKind) -> Self {
        match value {
            PollKind::Disclosed => Self::Disclosed,
            PollKind::Undisclosed => Self::Undisclosed,
        }
    }
}

impl From<RumaPollKind> for PollKind {
    fn from(value: RumaPollKind) -> Self {
        match value {
            RumaPollKind::Disclosed => Self::Disclosed,
            RumaPollKind::Undisclosed => Self::Undisclosed,
            _ => {
                info!("Unknown poll kind, defaulting to undisclosed");
                Self::Undisclosed
            }
        }
    }
}

/// Creates a [`RoomMessageEventContentWithoutRelation`] given a
/// [`MessageContent`] value.
#[matrix_sdk_ffi_macros::export]
pub fn content_without_relation_from_message(
    message: MessageContent,
) -> Result<Arc<RoomMessageEventContentWithoutRelation>, ClientError> {
    let msg_type = message.msg_type.try_into()?;
    Ok(Arc::new(RoomMessageEventContentWithoutRelation::new(msg_type)))
}
