/*
 * Copyright (c) 2026 Ankit Chaubey <ankitchaubey.dev@gmail.com>
 * https://github.com/ankit-chaubey
 *
 * Project: ferogram
 * Website: https://ferogram.dev
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your option.
 * This file may not be copied, modified, or distributed except according
 * to those terms.
 */

//! Inner TL layer of Telegram secret chats.
//!
//! These types live inside the encrypted `data` field of `messages.sendEncrypted`.
//! They are not part of the outer MTProto TL schema, so they are serialized and
//! parsed by hand here rather than by `ferogram-tl-types`.

use rand::Rng;

use crate::error::E2eError;

// Constructor IDs from the secret chat TL schema (https://core.telegram.org/schema/end-to-end).

/// Constructor ID of `decryptedMessage` (layer 73 and above).
pub const ID_DECRYPTED_MESSAGE: u32 = 0x91cc4674;

/// Constructor ID of `decryptedMessage` before layer 73.
pub const ID_DECRYPTED_MESSAGE_OLD: u32 = 0x36b091de;

/// Constructor ID of `decryptedMessageService` (layer 17 and above).
pub const ID_DECRYPTED_MESSAGE_SERVICE: u32 = 0x73164160;

/// Constructor ID of `decryptedMessageService` (layer 8).
pub const ID_DECRYPTED_MESSAGE_SERVICE8: u32 = 0xaa48327d;

/// Constructor ID of `decryptedMessageLayer`, the wrapper around every message from layer 17 on.
pub const ID_DECRYPTED_MESSAGE_LAYER: u32 = 0x1be31789;

/// Constructor ID of `decryptedMessageActionNotifyLayer`.
pub const ID_ACTION_NOTIFY_LAYER: u32 = 0xf3048883;

/// Constructor ID of `decryptedMessageActionRequestKey`.
pub const ID_ACTION_REQUEST_KEY: u32 = 0xf3c9611b;

/// Constructor ID of `decryptedMessageActionAcceptKey`.
pub const ID_ACTION_ACCEPT_KEY: u32 = 0x6fe1735b;

/// Constructor ID of `decryptedMessageActionAbortKey`.
pub const ID_ACTION_ABORT_KEY: u32 = 0xdd05ec6b;

/// Constructor ID of `decryptedMessageActionCommitKey`.
pub const ID_ACTION_COMMIT_KEY: u32 = 0xec2e0b9b;

/// Constructor ID of `decryptedMessageActionNoop`.
pub const ID_ACTION_NOOP: u32 = 0xa82fdd63;

/// Constructor ID of `decryptedMessageActionSetMessageTTL`.
pub const ID_ACTION_SET_TTL: u32 = 0xa1733aec;

/// Constructor ID of `decryptedMessageActionResend`.
pub const ID_ACTION_RESEND: u32 = 0x511110b0;

/// Constructor ID of `decryptedMessageActionReadMessages` (parsed only).
pub const ID_ACTION_READ_MESSAGES: u32 = 0x0c4f40be;

/// Constructor ID of `decryptedMessageActionDeleteMessages` (parsed only).
pub const ID_ACTION_DELETE_MESSAGES: u32 = 0x65614304;

/// Constructor ID of `decryptedMessageActionScreenshotMessages` (parsed only).
pub const ID_ACTION_SCREENSHOT_MESSAGES: u32 = 0x8ac1f475;

/// Constructor ID of `decryptedMessageActionFlushHistory` (parsed only).
pub const ID_ACTION_FLUSH_HISTORY: u32 = 0x6719e45c;

/// Constructor ID of `decryptedMessageActionTyping` (parsed only).
pub const ID_ACTION_TYPING: u32 = 0xccb27641;

/// Constructor ID of `decryptedMessageMediaEmpty`.
pub const ID_MEDIA_EMPTY: u32 = 0x089f5c4a;

/// Constructor ID of `decryptedMessageMediaVenue` (layer 45).
pub const ID_MEDIA_VENUE: u32 = 0x8a0df56f;

/// Constructor ID of `decryptedMessageMediaPhoto` (layer 45 and above).
pub const ID_MEDIA_PHOTO: u32 = 0xf1fa8d78;

/// Constructor ID of `decryptedMessageMediaPhoto` (layer 8).
pub const ID_MEDIA_PHOTO_OLD: u32 = 0x32798a8c;

/// Constructor ID of `decryptedMessageMediaDocument` (layer 143 and above).
pub const ID_MEDIA_DOCUMENT: u32 = 0x6abd9782;

/// Constructor ID of `decryptedMessageMediaDocument` (layer 8).
pub const ID_MEDIA_DOCUMENT_OLD: u32 = 0xb095434b;

/// Constructor ID of `decryptedMessageMediaGeoPoint`.
pub const ID_MEDIA_GEO: u32 = 0x35480a59;

/// Constructor ID of `decryptedMessageMediaContact`.
pub const ID_MEDIA_CONTACT: u32 = 0x588a0a97;

/// Constructor ID of `decryptedMessageMediaAudio` (layer 8).
pub const ID_MEDIA_AUDIO_L8: u32 = 0x6080758f;

/// Constructor ID of `decryptedMessageMediaAudio` (layer 17).
pub const ID_MEDIA_AUDIO_L17: u32 = 0x57e0a9cb;

/// Constructor ID of `decryptedMessageMediaVideo` (layer 8).
pub const ID_MEDIA_VIDEO_L8: u32 = 0x4cee6ef3;

/// Constructor ID of `decryptedMessageMediaVideo` (layer 17).
pub const ID_MEDIA_VIDEO_L17: u32 = 0x524a415d;

/// Constructor ID of `decryptedMessageMediaVideo` (layer 45).
pub const ID_MEDIA_VIDEO_L45: u32 = 0x970c8c0e;

/// Constructor ID of `documentAttributeImageSize`.
pub const ID_ATTR_IMAGE_SIZE: u32 = 0x6c37c15c;

/// Constructor ID of `documentAttributeAnimated`.
pub const ID_ATTR_ANIMATED: u32 = 0x11b58939;

/// Constructor ID of `documentAttributeSticker` as used in secret chats (layer 17 and above).
pub const ID_ATTR_STICKER: u32 = 0x3a556302;

/// Constructor ID of `documentAttributeVideo`.
pub const ID_ATTR_VIDEO: u32 = 0x0ef02ce6;

/// Constructor ID of `documentAttributeAudio`.
pub const ID_ATTR_AUDIO: u32 = 0x9852f9c6;

/// Constructor ID of `documentAttributeFilename`.
pub const ID_ATTR_FILENAME: u32 = 0x15590068;

/// TL `vector` constructor ID.
const TL_VECTOR: u32 = 0x1cb5c415;

/// A service action carried inside a `decryptedMessageService`.
#[derive(Debug, Clone)]
pub enum DecryptedAction {
    /// Announces the sender's supported layer.
    NotifyLayer {
        /// Highest layer the sender supports.
        layer: i32,
    },
    /// Starts a key re-exchange.
    RequestKey {
        /// Identifier of this exchange.
        exchange_id: i64,
        /// Sender's DH public value `g_a`.
        g_a: Vec<u8>,
    },
    /// Accepts a key re-exchange.
    AcceptKey {
        /// Identifier of this exchange.
        exchange_id: i64,
        /// Sender's DH public value `g_b`.
        g_b: Vec<u8>,
        /// Fingerprint of the key that results from the exchange.
        key_fingerprint: i64,
    },
    /// Aborts a key re-exchange.
    AbortKey {
        /// Identifier of the aborted exchange.
        exchange_id: i64,
    },
    /// Commits to the new key after a successful re-exchange.
    CommitKey {
        /// Identifier of this exchange.
        exchange_id: i64,
        /// Fingerprint of the new key.
        key_fingerprint: i64,
    },
    /// Does nothing; used to acknowledge sequence numbers.
    Noop,
    /// Sets the self-destruct timer for following messages.
    SetTtl {
        /// New TTL in seconds (0 disables it).
        ttl_seconds: i32,
    },
    /// Asks the peer to resend messages in a sequence-number range.
    Resend {
        /// First raw sequence number to resend.
        start_seq_no: i32,
        /// Last raw sequence number to resend.
        end_seq_no: i32,
    },
    /// The peer read these messages. Parsed only, never sent.
    ReadMessages {
        /// `random_id`s of the messages that were read.
        random_ids: Vec<i64>,
    },
    /// The peer deleted these messages. Parsed only, never sent.
    DeleteMessages {
        /// `random_id`s of the deleted messages.
        random_ids: Vec<i64>,
    },
    /// The peer took a screenshot of these messages. Parsed only, never sent.
    ScreenshotMessages {
        /// `random_id`s of the captured messages.
        random_ids: Vec<i64>,
    },
    /// The peer cleared the chat history. Parsed only, never sent.
    FlushHistory,
    /// The peer is typing or doing another chat action. Parsed only, never sent.
    Typing {
        /// Constructor ID of the `SendMessageAction` the peer reported.
        action_ctor: u32,
    },
    /// An action this crate does not know.
    Unknown {
        /// Its constructor ID.
        ctor: u32,
    },
}

/// Document attributes carried inside `DecryptedMedia::Document`.
#[derive(Debug, Clone)]
pub enum DocumentAttribute {
    /// Image dimensions in pixels.
    ImageSize {
        /// Width.
        w: i32,
        /// Height.
        h: i32,
    },
    /// The document is an animation (GIF).
    Animated,
    /// The document is a sticker.
    Sticker {
        /// Emoji the sticker represents.
        alt: String,
    },
    /// Video metadata.
    Video {
        /// Duration in seconds.
        duration: i32,
        /// Width in pixels.
        w: i32,
        /// Height in pixels.
        h: i32,
    },
    /// Audio metadata.
    Audio {
        /// Duration in seconds.
        duration: i32,
        /// Track title, if present.
        title: Option<String>,
        /// Performer, if present.
        performer: Option<String>,
    },
    /// Original file name.
    FileName {
        /// The file name.
        file_name: String,
    },
    /// An attribute this crate does not know.
    Unknown {
        /// Its constructor ID.
        ctor: u32,
    },
}

/// Media payload of a decrypted message.
///
/// For `Photo` and `Document`, `key` and `iv` are the one-time AES-256-IGE key and
/// IV (32 bytes each, fully used by IGE) of the separately uploaded encrypted file.
#[derive(Debug, Clone)]
pub enum DecryptedMedia {
    /// No media.
    Empty,
    /// A photo.
    Photo {
        /// JPEG thumbnail bytes.
        thumb: Vec<u8>,
        /// Thumbnail width.
        thumb_w: i32,
        /// Thumbnail height.
        thumb_h: i32,
        /// Photo width in pixels.
        w: i32,
        /// Photo height in pixels.
        h: i32,
        /// Size of the plaintext file in bytes.
        size: i32,
        /// File encryption key (32 bytes).
        key: Vec<u8>,
        /// File encryption IV (32 bytes).
        iv: Vec<u8>,
        /// Caption.
        caption: String,
    },
    /// A generic document.
    Document {
        /// Thumbnail bytes.
        thumb: Vec<u8>,
        /// Thumbnail width.
        thumb_w: i32,
        /// Thumbnail height.
        thumb_h: i32,
        /// MIME type.
        mime_type: String,
        /// Size of the plaintext file in bytes.
        size: i64,
        /// File encryption key (32 bytes).
        key: Vec<u8>,
        /// File encryption IV (32 bytes).
        iv: Vec<u8>,
        /// Document attributes.
        attributes: Vec<DocumentAttribute>,
        /// Caption.
        caption: String,
    },
    /// A location.
    GeoPoint {
        /// Latitude.
        lat: f64,
        /// Longitude.
        long: f64,
    },
    /// A shared contact.
    Contact {
        /// Phone number.
        phone_number: String,
        /// First name.
        first_name: String,
        /// Last name.
        last_name: String,
        /// Telegram user id of the contact.
        user_id: i32,
    },
    /// A media type this crate does not know.
    Unknown {
        /// Its constructor ID.
        ctor: u32,
    },
}

/// A decrypted message as carried inside [`DecryptedMessageLayer`].
#[derive(Debug, Clone)]
pub enum DecryptedMessage {
    /// A text message, optionally with media.
    Text {
        /// Client-generated unique id.
        random_id: i64,
        /// Self-destruct timer in seconds (0 = none).
        ttl: i32,
        /// Message text.
        message: String,
        /// Attached media.
        media: DecryptedMedia,
    },
    /// A service message carrying an action.
    Service {
        /// Client-generated unique id.
        random_id: i64,
        /// The action.
        action: DecryptedAction,
    },
    /// An unrecognized constructor. The raw bytes are kept for logging and are
    /// not forwarded to the caller.
    Unknown {
        /// Its constructor ID.
        ctor: u32,
        /// The undecoded bytes.
        raw: Vec<u8>,
    },
}

/// The outer wrapper of every secret chat message (layer 17 and above).
#[derive(Debug, Clone)]
pub struct DecryptedMessageLayer {
    /// Random filler (15, 19 or 23 bytes) that varies the payload length.
    pub random_bytes: Vec<u8>,
    /// Layer the sender uses.
    pub layer: i32,
    /// Number of messages the sender has received, as a wire sequence number.
    pub in_seq_no: i32,
    /// Number of messages the sender has sent, as a wire sequence number.
    pub out_seq_no: i32,
    /// The wrapped message.
    pub message: DecryptedMessage,
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn write_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 254 {
        buf.push(len as u8);
        buf.extend_from_slice(data);
        let pad = (4 - (1 + len) % 4) % 4;
        for _ in 0..pad {
            buf.push(0);
        }
    } else {
        buf.push(0xfe);
        buf.push((len & 0xff) as u8);
        buf.push(((len >> 8) & 0xff) as u8);
        buf.push(((len >> 16) & 0xff) as u8);
        buf.extend_from_slice(data);
        let pad = (4 - (4 + len) % 4) % 4;
        for _ in 0..pad {
            buf.push(0);
        }
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_bytes(buf, s.as_bytes());
}

fn need(data: &[u8], offset: usize, n: usize) -> Result<(), E2eError> {
    if offset + n > data.len() {
        Err(E2eError::TlDeserializationFailed)
    } else {
        Ok(())
    }
}

fn read_u32(data: &[u8], offset: &mut usize) -> Result<u32, E2eError> {
    need(data, *offset, 4)?;
    let mut b = [0u8; 4];
    b.copy_from_slice(&data[*offset..*offset + 4]);
    *offset += 4;
    Ok(u32::from_le_bytes(b))
}

fn read_i32(data: &[u8], offset: &mut usize) -> Result<i32, E2eError> {
    Ok(read_u32(data, offset)? as i32)
}

fn read_i64(data: &[u8], offset: &mut usize) -> Result<i64, E2eError> {
    need(data, *offset, 8)?;
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[*offset..*offset + 8]);
    *offset += 8;
    Ok(i64::from_le_bytes(b))
}

fn read_f64(data: &[u8], offset: &mut usize) -> Result<f64, E2eError> {
    need(data, *offset, 8)?;
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[*offset..*offset + 8]);
    *offset += 8;
    Ok(f64::from_le_bytes(b))
}

fn read_bytes(data: &[u8], offset: &mut usize) -> Result<Vec<u8>, E2eError> {
    need(data, *offset, 1)?;
    let first = data[*offset];
    if first < 254 {
        let len = first as usize;
        *offset += 1;
        need(data, *offset, len)?;
        let out = data[*offset..*offset + len].to_vec();
        *offset += len;
        let pad = (4 - (1 + len) % 4) % 4;
        *offset += pad;
        Ok(out)
    } else if first == 0xfe {
        need(data, *offset, 4)?;
        let len = (data[*offset + 1] as usize)
            | ((data[*offset + 2] as usize) << 8)
            | ((data[*offset + 3] as usize) << 16);
        *offset += 4;
        need(data, *offset, len)?;
        let out = data[*offset..*offset + len].to_vec();
        *offset += len;
        let pad = (4 - (4 + len) % 4) % 4;
        *offset += pad;
        Ok(out)
    } else {
        Err(E2eError::TlDeserializationFailed)
    }
}

fn read_string(data: &[u8], offset: &mut usize) -> Result<String, E2eError> {
    String::from_utf8(read_bytes(data, offset)?).map_err(|_| E2eError::TlDeserializationFailed)
}

/// Read a `Vector<long>` (boxed: ctor + i32 count + i64 elements).
fn read_vector_long(data: &[u8], offset: &mut usize) -> Result<Vec<i64>, E2eError> {
    if read_u32(data, offset)? != TL_VECTOR {
        return Err(E2eError::TlDeserializationFailed);
    }
    let count = read_i32(data, offset)?;
    if !(0..=65536).contains(&count) {
        return Err(E2eError::TlDeserializationFailed);
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(read_i64(data, offset)?);
    }
    Ok(out)
}

fn serialize_doc_attr(buf: &mut Vec<u8>, attr: &DocumentAttribute) {
    match attr {
        DocumentAttribute::ImageSize { w, h } => {
            write_u32(buf, ID_ATTR_IMAGE_SIZE);
            write_i32(buf, *w);
            write_i32(buf, *h);
        }
        DocumentAttribute::Animated => {
            write_u32(buf, ID_ATTR_ANIMATED);
        }
        DocumentAttribute::Sticker { alt } => {
            // #3a556302: alt:string stickerset:InputStickerSet, no flags field
            write_u32(buf, ID_ATTR_STICKER);
            write_string(buf, alt);
            write_u32(buf, 0xffb62b95); // inputStickerSetEmpty
        }
        DocumentAttribute::Video { duration, w, h } => {
            write_u32(buf, ID_ATTR_VIDEO);
            write_u32(buf, 0u32); // flags: none
            write_i32(buf, *duration);
            write_i32(buf, *w);
            write_i32(buf, *h);
        }
        DocumentAttribute::Audio {
            duration,
            title,
            performer,
        } => {
            write_u32(buf, ID_ATTR_AUDIO);
            let mut flags: u32 = 0;
            if title.is_some() {
                flags |= 1;
            }
            if performer.is_some() {
                flags |= 2;
            }
            write_u32(buf, flags);
            write_i32(buf, *duration);
            if let Some(t) = title {
                write_string(buf, t);
            }
            if let Some(p) = performer {
                write_string(buf, p);
            }
        }
        DocumentAttribute::FileName { file_name } => {
            write_u32(buf, ID_ATTR_FILENAME);
            write_string(buf, file_name);
        }
        DocumentAttribute::Unknown { .. } => {
            // Never send an unknown attribute.
        }
    }
}

fn deserialize_doc_attr(data: &[u8], offset: &mut usize) -> Result<DocumentAttribute, E2eError> {
    let ctor = read_u32(data, offset)?;
    match ctor {
        ID_ATTR_IMAGE_SIZE => Ok(DocumentAttribute::ImageSize {
            w: read_i32(data, offset)?,
            h: read_i32(data, offset)?,
        }),
        ID_ATTR_ANIMATED => Ok(DocumentAttribute::Animated),
        // layer 8 sticker: no fields at all
        0xfb0a5727 => Ok(DocumentAttribute::Sticker { alt: String::new() }),
        ID_ATTR_STICKER => {
            // #3a556302: alt:string stickerset:InputStickerSet, no flags field
            let alt = read_string(data, offset)?;
            // Skip the InputStickerSet field (boxed, variable size).
            let set_ctor = read_u32(data, offset)?;
            match set_ctor {
                0xffb62b95 => {} // inputStickerSetEmpty: no fields
                0x861cc8a0 => {
                    let _ = read_string(data, offset);
                } // inputStickerSetShortName: short_name:string
                _ => {}
            }
            Ok(DocumentAttribute::Sticker { alt })
        }
        ID_ATTR_VIDEO => {
            let _flags = read_u32(data, offset)?;
            let duration = read_i32(data, offset)?;
            let w = read_i32(data, offset)?;
            let h = read_i32(data, offset)?;
            Ok(DocumentAttribute::Video { duration, w, h })
        }
        ID_ATTR_AUDIO => {
            let flags = read_u32(data, offset)?;
            let duration = read_i32(data, offset)?;
            let title = if flags & (1 << 0) != 0 {
                Some(read_string(data, offset)?)
            } else {
                None
            };
            let performer = if flags & (1 << 1) != 0 {
                Some(read_string(data, offset)?)
            } else {
                None
            };
            // waveform:flags.2?bytes must be consumed even though we don't expose it
            if flags & (1 << 2) != 0 {
                let _ = read_bytes(data, offset)?;
            }
            Ok(DocumentAttribute::Audio {
                duration,
                title,
                performer,
            })
        }
        ID_ATTR_FILENAME => Ok(DocumentAttribute::FileName {
            file_name: read_string(data, offset)?,
        }),
        _ => Ok(DocumentAttribute::Unknown { ctor }),
    }
}

/// Serialize `media` into `buf` (constructor ID followed by its fields).
pub fn serialize_media(buf: &mut Vec<u8>, media: &DecryptedMedia) {
    match media {
        DecryptedMedia::Empty => {
            write_u32(buf, ID_MEDIA_EMPTY);
        }
        DecryptedMedia::Photo {
            thumb,
            thumb_w,
            thumb_h,
            w,
            h,
            size,
            key,
            iv,
            caption,
        } => {
            write_u32(buf, ID_MEDIA_PHOTO);
            write_bytes(buf, thumb);
            write_i32(buf, *thumb_w);
            write_i32(buf, *thumb_h);
            write_i32(buf, *w);
            write_i32(buf, *h);
            write_i32(buf, *size);
            write_bytes(buf, key);
            write_bytes(buf, iv);
            write_string(buf, caption);
        }
        DecryptedMedia::Document {
            thumb,
            thumb_w,
            thumb_h,
            mime_type,
            size,
            key,
            iv,
            attributes,
            caption,
        } => {
            write_u32(buf, ID_MEDIA_DOCUMENT);
            write_bytes(buf, thumb);
            write_i32(buf, *thumb_w);
            write_i32(buf, *thumb_h);
            write_string(buf, mime_type);
            write_i64(buf, *size);
            write_bytes(buf, key);
            write_bytes(buf, iv);
            write_u32(buf, TL_VECTOR);
            write_i32(buf, attributes.len() as i32);
            for attr in attributes {
                serialize_doc_attr(buf, attr);
            }
            write_string(buf, caption);
        }
        DecryptedMedia::GeoPoint { lat, long } => {
            write_u32(buf, ID_MEDIA_GEO);
            buf.extend_from_slice(&lat.to_le_bytes());
            buf.extend_from_slice(&long.to_le_bytes());
        }
        DecryptedMedia::Contact {
            phone_number,
            first_name,
            last_name,
            user_id,
        } => {
            write_u32(buf, ID_MEDIA_CONTACT);
            write_string(buf, phone_number);
            write_string(buf, first_name);
            write_string(buf, last_name);
            write_i32(buf, *user_id);
        }
        DecryptedMedia::Unknown { .. } => {
            write_u32(buf, ID_MEDIA_EMPTY);
        }
    }
}

/// Parse a media object from `data` starting at `*offset`, advancing the offset.
pub fn deserialize_media(data: &[u8], offset: &mut usize) -> Result<DecryptedMedia, E2eError> {
    if *offset + 4 > data.len() {
        return Ok(DecryptedMedia::Empty);
    }
    let ctor = read_u32(data, offset)?;
    match ctor {
        ID_MEDIA_EMPTY => Ok(DecryptedMedia::Empty),

        ID_MEDIA_PHOTO => {
            let thumb = read_bytes(data, offset)?;
            let thumb_w = read_i32(data, offset)?;
            let thumb_h = read_i32(data, offset)?;
            let w = read_i32(data, offset)?;
            let h = read_i32(data, offset)?;
            let size = read_i32(data, offset)?;
            let key = read_bytes(data, offset)?;
            let iv = read_bytes(data, offset)?;
            let caption = read_string(data, offset).unwrap_or_default();
            Ok(DecryptedMedia::Photo {
                thumb,
                thumb_w,
                thumb_h,
                w,
                h,
                size,
                key,
                iv,
                caption,
            })
        }

        ID_MEDIA_PHOTO_OLD => {
            // layer 8 photo: no caption field
            let thumb = read_bytes(data, offset)?;
            let thumb_w = read_i32(data, offset)?;
            let thumb_h = read_i32(data, offset)?;
            let w = read_i32(data, offset)?;
            let h = read_i32(data, offset)?;
            let size = read_i32(data, offset)?;
            let key = read_bytes(data, offset)?;
            let iv = read_bytes(data, offset)?;
            Ok(DecryptedMedia::Photo {
                thumb,
                thumb_w,
                thumb_h,
                w,
                h,
                size,
                key,
                iv,
                caption: String::new(),
            })
        }

        ID_MEDIA_DOCUMENT => {
            let thumb = read_bytes(data, offset)?;
            let thumb_w = read_i32(data, offset)?;
            let thumb_h = read_i32(data, offset)?;
            let mime_type = read_string(data, offset)?;
            let size = read_i64(data, offset)?;
            let key = read_bytes(data, offset)?;
            let iv = read_bytes(data, offset)?;
            let attributes = if *offset + 4 <= data.len() {
                let vec_ctor = read_u32(data, offset)?;
                if vec_ctor == TL_VECTOR {
                    let count = read_i32(data, offset)?.clamp(0, 64) as usize;
                    let mut attrs = Vec::with_capacity(count);
                    for _ in 0..count {
                        match deserialize_doc_attr(data, offset) {
                            Ok(a) => attrs.push(a),
                            Err(_) => break,
                        }
                    }
                    attrs
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let caption = read_string(data, offset).unwrap_or_default();
            Ok(DecryptedMedia::Document {
                thumb,
                thumb_w,
                thumb_h,
                mime_type,
                size,
                key,
                iv,
                attributes,
                caption,
            })
        }

        ID_MEDIA_DOCUMENT_OLD => {
            // layer 8 document: file_name + mime_type + size:int, no attributes, no caption
            let thumb = read_bytes(data, offset)?;
            let thumb_w = read_i32(data, offset)?;
            let thumb_h = read_i32(data, offset)?;
            let _file_name = read_string(data, offset)?; // separate field in layer 8
            let mime_type = read_string(data, offset)?;
            let size = read_i32(data, offset)? as i64;
            let key = read_bytes(data, offset)?;
            let iv = read_bytes(data, offset)?;
            Ok(DecryptedMedia::Document {
                thumb,
                thumb_w,
                thumb_h,
                mime_type,
                size,
                key,
                iv,
                attributes: Vec::new(),
                caption: String::new(),
            })
        }

        // decryptedMessageMediaDocument#7afe8ae2, layer 45-142
        // thumb:bytes thumb_w:int thumb_h:int mime_type:string size:int key:bytes iv:bytes attributes:Vector<DocumentAttribute> caption:string
        0x7afe8ae2 => {
            let thumb = read_bytes(data, offset)?;
            let thumb_w = read_i32(data, offset)?;
            let thumb_h = read_i32(data, offset)?;
            let mime_type = read_string(data, offset)?;
            let size = read_i32(data, offset)? as i64;
            let key = read_bytes(data, offset)?;
            let iv = read_bytes(data, offset)?;
            let attributes = if *offset + 4 <= data.len() {
                let vec_ctor = read_u32(data, offset)?;
                if vec_ctor == TL_VECTOR {
                    let count = read_i32(data, offset)?.clamp(0, 64) as usize;
                    let mut attrs = Vec::with_capacity(count);
                    for _ in 0..count {
                        match deserialize_doc_attr(data, offset) {
                            Ok(a) => attrs.push(a),
                            Err(_) => break,
                        }
                    }
                    attrs
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let caption = read_string(data, offset).unwrap_or_default();
            Ok(DecryptedMedia::Document {
                thumb,
                thumb_w,
                thumb_h,
                mime_type,
                size,
                key,
                iv,
                attributes,
                caption,
            })
        }

        ID_MEDIA_GEO => {
            let lat = read_f64(data, offset)?;
            let long = read_f64(data, offset)?;
            Ok(DecryptedMedia::GeoPoint { lat, long })
        }

        ID_MEDIA_CONTACT => {
            let phone_number = read_string(data, offset)?;
            let first_name = read_string(data, offset)?;
            let last_name = read_string(data, offset)?;
            let user_id = read_i32(data, offset)?;
            Ok(DecryptedMedia::Contact {
                phone_number,
                first_name,
                last_name,
                user_id,
            })
        }

        // Old audio/video types: just note the ctor, no fields we care about.
        ID_MEDIA_AUDIO_L8 | ID_MEDIA_AUDIO_L17 | ID_MEDIA_VIDEO_L8 | ID_MEDIA_VIDEO_L17
        | ID_MEDIA_VIDEO_L45 => Ok(DecryptedMedia::Unknown { ctor }),

        // Venue: parsed but not exposed as a rich type yet.
        ID_MEDIA_VENUE => Ok(DecryptedMedia::Unknown { ctor }),

        _ => Ok(DecryptedMedia::Unknown { ctor }),
    }
}

fn serialize_action(buf: &mut Vec<u8>, action: &DecryptedAction) {
    match action {
        DecryptedAction::NotifyLayer { layer } => {
            write_u32(buf, ID_ACTION_NOTIFY_LAYER);
            write_i32(buf, *layer);
        }
        DecryptedAction::RequestKey { exchange_id, g_a } => {
            write_u32(buf, ID_ACTION_REQUEST_KEY);
            write_i64(buf, *exchange_id);
            write_bytes(buf, g_a);
        }
        DecryptedAction::AcceptKey {
            exchange_id,
            g_b,
            key_fingerprint,
        } => {
            write_u32(buf, ID_ACTION_ACCEPT_KEY);
            write_i64(buf, *exchange_id);
            write_bytes(buf, g_b);
            write_i64(buf, *key_fingerprint);
        }
        DecryptedAction::AbortKey { exchange_id } => {
            write_u32(buf, ID_ACTION_ABORT_KEY);
            write_i64(buf, *exchange_id);
        }
        DecryptedAction::CommitKey {
            exchange_id,
            key_fingerprint,
        } => {
            write_u32(buf, ID_ACTION_COMMIT_KEY);
            write_i64(buf, *exchange_id);
            write_i64(buf, *key_fingerprint);
        }
        DecryptedAction::Noop => {
            write_u32(buf, ID_ACTION_NOOP);
        }
        DecryptedAction::SetTtl { ttl_seconds } => {
            write_u32(buf, ID_ACTION_SET_TTL);
            write_i32(buf, *ttl_seconds);
        }
        DecryptedAction::Resend {
            start_seq_no,
            end_seq_no,
        } => {
            write_u32(buf, ID_ACTION_RESEND);
            write_i32(buf, *start_seq_no);
            write_i32(buf, *end_seq_no);
        }
        // Peer-only actions: should never be serialized by us.
        DecryptedAction::ReadMessages { .. }
        | DecryptedAction::DeleteMessages { .. }
        | DecryptedAction::ScreenshotMessages { .. }
        | DecryptedAction::FlushHistory
        | DecryptedAction::Typing { .. }
        | DecryptedAction::Unknown { .. } => {}
    }
}

fn deserialize_action(data: &[u8], offset: &mut usize) -> Result<DecryptedAction, E2eError> {
    let ctor = read_u32(data, offset)?;
    match ctor {
        ID_ACTION_NOTIFY_LAYER => Ok(DecryptedAction::NotifyLayer {
            layer: read_i32(data, offset)?,
        }),
        ID_ACTION_REQUEST_KEY => Ok(DecryptedAction::RequestKey {
            exchange_id: read_i64(data, offset)?,
            g_a: read_bytes(data, offset)?,
        }),
        ID_ACTION_ACCEPT_KEY => Ok(DecryptedAction::AcceptKey {
            exchange_id: read_i64(data, offset)?,
            g_b: read_bytes(data, offset)?,
            key_fingerprint: read_i64(data, offset)?,
        }),
        ID_ACTION_ABORT_KEY => Ok(DecryptedAction::AbortKey {
            exchange_id: read_i64(data, offset)?,
        }),
        ID_ACTION_COMMIT_KEY => Ok(DecryptedAction::CommitKey {
            exchange_id: read_i64(data, offset)?,
            key_fingerprint: read_i64(data, offset)?,
        }),
        ID_ACTION_NOOP => Ok(DecryptedAction::Noop),
        ID_ACTION_SET_TTL => Ok(DecryptedAction::SetTtl {
            ttl_seconds: read_i32(data, offset)?,
        }),
        ID_ACTION_RESEND => Ok(DecryptedAction::Resend {
            start_seq_no: read_i32(data, offset)?,
            end_seq_no: read_i32(data, offset)?,
        }),
        ID_ACTION_READ_MESSAGES => Ok(DecryptedAction::ReadMessages {
            random_ids: read_vector_long(data, offset)?,
        }),
        ID_ACTION_DELETE_MESSAGES => Ok(DecryptedAction::DeleteMessages {
            random_ids: read_vector_long(data, offset)?,
        }),
        ID_ACTION_SCREENSHOT_MESSAGES => Ok(DecryptedAction::ScreenshotMessages {
            random_ids: read_vector_long(data, offset)?,
        }),
        ID_ACTION_FLUSH_HISTORY => Ok(DecryptedAction::FlushHistory),
        ID_ACTION_TYPING => Ok(DecryptedAction::Typing {
            action_ctor: read_u32(data, offset)?,
        }),
        _ => Ok(DecryptedAction::Unknown { ctor }),
    }
}

fn serialize_message(buf: &mut Vec<u8>, msg: &DecryptedMessage) {
    match msg {
        DecryptedMessage::Text {
            random_id,
            ttl,
            message,
            media,
        } => {
            write_u32(buf, ID_DECRYPTED_MESSAGE);
            // flags bit 9 = media present. Must be set iff media is non-empty;
            // tdesktop (and any spec-compliant peer) reads media only when bit 9 is set.
            let has_media = !matches!(media, DecryptedMedia::Empty);
            let flags: u32 = if has_media { 1 << 9 } else { 0 };
            write_u32(buf, flags);
            write_i64(buf, *random_id);
            write_i32(buf, *ttl);
            write_string(buf, message);
            if has_media {
                serialize_media(buf, media);
            }
        }
        DecryptedMessage::Service { random_id, action } => {
            write_u32(buf, ID_DECRYPTED_MESSAGE_SERVICE);
            write_i64(buf, *random_id);
            serialize_action(buf, action);
        }
        DecryptedMessage::Unknown { .. } => {
            // Should not be serialized by us.
        }
    }
}

fn deserialize_message(data: &[u8], offset: &mut usize) -> Result<DecryptedMessage, E2eError> {
    let ctor = read_u32(data, offset)?;
    match ctor {
        ID_DECRYPTED_MESSAGE => {
            let flags = read_u32(data, offset)?;
            let random_id = read_i64(data, offset)?;
            let ttl = read_i32(data, offset)?;
            let message = read_string(data, offset)?;
            // bit 9 = media present
            let media = if flags & (1 << 9) != 0 {
                deserialize_media(data, offset).unwrap_or(DecryptedMedia::Empty)
            } else {
                DecryptedMedia::Empty
            };
            Ok(DecryptedMessage::Text {
                random_id,
                ttl,
                message,
                media,
            })
        }
        ID_DECRYPTED_MESSAGE_OLD => {
            // layer 45: flags:# random_id:long ttl:int message:string media:flags.9?...
            let flags = read_u32(data, offset)?;
            let random_id = read_i64(data, offset)?;
            let ttl = read_i32(data, offset)?;
            let message = read_string(data, offset)?;
            let media = if flags & (1 << 9) != 0 {
                deserialize_media(data, offset).unwrap_or(DecryptedMedia::Empty)
            } else {
                DecryptedMedia::Empty
            };
            Ok(DecryptedMessage::Text {
                random_id,
                ttl,
                message,
                media,
            })
        }
        // layer 8 text: random_id:long random_bytes:bytes message:string media:DecryptedMessageMedia
        0x1f814f1f => {
            let random_id = read_i64(data, offset)?;
            let _random_bytes = read_bytes(data, offset)?;
            let message = read_string(data, offset)?;
            let media = deserialize_media(data, offset).unwrap_or(DecryptedMedia::Empty);
            Ok(DecryptedMessage::Text {
                random_id,
                ttl: 0,
                message,
                media,
            })
        }
        // layer 17 text: random_id:long ttl:int message:string media:DecryptedMessageMedia (no flags, unconditional media)
        0x204d3878 => {
            let random_id = read_i64(data, offset)?;
            let ttl = read_i32(data, offset)?;
            let message = read_string(data, offset)?;
            let media = deserialize_media(data, offset).unwrap_or(DecryptedMedia::Empty);
            Ok(DecryptedMessage::Text {
                random_id,
                ttl,
                message,
                media,
            })
        }
        ID_DECRYPTED_MESSAGE_SERVICE => {
            let random_id = read_i64(data, offset)?;
            let action = deserialize_action(data, offset)?;
            Ok(DecryptedMessage::Service { random_id, action })
        }
        // layer 8 service: random_id:long random_bytes:bytes action:DecryptedMessageAction
        ID_DECRYPTED_MESSAGE_SERVICE8 => {
            let random_id = read_i64(data, offset)?;
            let _random_bytes = read_bytes(data, offset)?;
            let action = deserialize_action(data, offset)?;
            Ok(DecryptedMessage::Service { random_id, action })
        }
        _ => {
            let raw = data[*offset..].to_vec();
            *offset = data.len();
            Ok(DecryptedMessage::Unknown { ctor, raw })
        }
    }
}

/// Serialize a [`DecryptedMessageLayer`] to bytes, ready for `crypto::encrypt_message`.
pub fn serialize_message_layer(layer: &DecryptedMessageLayer) -> Result<Vec<u8>, E2eError> {
    let mut buf = Vec::new();
    write_u32(&mut buf, ID_DECRYPTED_MESSAGE_LAYER);
    write_bytes(&mut buf, &layer.random_bytes);
    write_i32(&mut buf, layer.layer);
    write_i32(&mut buf, layer.in_seq_no);
    write_i32(&mut buf, layer.out_seq_no);
    serialize_message(&mut buf, &layer.message);
    Ok(buf)
}

/// Parse a [`DecryptedMessageLayer`] from decrypted bytes.
pub fn deserialize_message_layer(data: &[u8]) -> Result<DecryptedMessageLayer, E2eError> {
    let mut offset = 0usize;
    if read_u32(data, &mut offset)? != ID_DECRYPTED_MESSAGE_LAYER {
        return Err(E2eError::TlDeserializationFailed);
    }
    let random_bytes = read_bytes(data, &mut offset)?;
    let layer = read_i32(data, &mut offset)?;
    let in_seq_no = read_i32(data, &mut offset)?;
    let out_seq_no = read_i32(data, &mut offset)?;
    let message = deserialize_message(data, &mut offset)?;
    Ok(DecryptedMessageLayer {
        random_bytes,
        layer,
        in_seq_no,
        out_seq_no,
        message,
    })
}

/// Serialize a layer-8 service message (no [`DecryptedMessageLayer`] wrapper).
///
/// Used for the initial `notify_layer` handshake.
pub fn serialize_layer8_service(
    random_id: i64,
    action: &DecryptedAction,
) -> Result<Vec<u8>, E2eError> {
    let mut buf = Vec::new();
    write_u32(&mut buf, ID_DECRYPTED_MESSAGE_SERVICE8);
    write_i64(&mut buf, random_id);
    let mut rb = [0u8; 15];
    rand::rng().fill_bytes(&mut rb);
    write_bytes(&mut buf, &rb);
    serialize_action(&mut buf, action);
    Ok(buf)
}

/// Build the `random_bytes` field of a [`DecryptedMessageLayer`].
///
/// The length is 15, 19 or 23 bytes, so payload sizes vary slightly.
pub fn make_random_bytes() -> Vec<u8> {
    let extra = (rand::random::<u8>() % 3) as usize * 4;
    let mut rb = vec![0u8; 15 + extra];
    rand::rng().fill_bytes(&mut rb);
    rb
}
