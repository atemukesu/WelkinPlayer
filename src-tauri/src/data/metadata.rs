// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Audio metadata extraction.
//!
//! Parsing happens on an in-memory buffer filled from a single HTTP `Range`
//! request (see `WebDavClient::get_range`), so metadata — including embedded
//! cover art — is read from the file head without downloading the whole file.

use std::borrow::Cow;
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};

use crate::core::error::AppError;

/// Text metadata returned to the frontend (also persisted to the metadata cache).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Track length in whole seconds, when the container advertises it.
    pub duration_secs: Option<u64>,
    /// Cache id of the generated cover thumbnail, when one was embedded.
    pub cover_hash: Option<String>,
}

/// Result of parsing a buffer: the text metadata plus the raw embedded cover.
pub struct ParsedMetadata {
    pub metadata: TrackMetadata,
    pub cover: Option<Vec<u8>>,
}

/// Full editable tag set, read from and written back to an audio file.
///
/// The frontend round-trips this object: [`read_tags`] fills it, the user edits
/// the fields, and [`write_tags`] applies it. `None` (or an empty string) clears
/// the corresponding tag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub disc_total: Option<u32>,
    /// Whether the file already embeds cover art (informational, for the UI).
    #[serde(default)]
    pub has_cover: bool,
}

/// Parse tags and properties from an audio file head.
pub fn parse(bytes: &[u8]) -> Result<ParsedMetadata, AppError> {
    let tagged = parse_tagged(bytes)?;

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let title = tag
        .and_then(|tag| tag.title())
        .map(|value| value.into_owned());
    let artist = tag
        .and_then(|tag| tag.artist())
        .map(|value| value.into_owned());
    let album = tag
        .and_then(|tag| tag.album())
        .map(|value| value.into_owned());

    let duration: Duration = tagged.properties().duration();
    let duration_secs = (duration.as_secs() > 0).then_some(duration.as_secs());

    // Cover art can live on any tag, so look through all of them.
    let cover = tagged
        .tags()
        .iter()
        .find_map(|tag| tag.pictures().first())
        .map(|picture| picture.data().to_vec());

    Ok(ParsedMetadata {
        metadata: TrackMetadata {
            title,
            artist,
            album,
            duration_secs,
            cover_hash: None,
        },
        cover,
    })
}

/// Decode embedded cover art and re-encode it as a small JPEG thumbnail.
pub fn encode_thumbnail(data: &[u8]) -> Result<Vec<u8>, AppError> {
    use image::codecs::jpeg::JpegEncoder;

    let image = image::load_from_memory(data)
        .map_err(|error| AppError::Other(format!("cover decode error: {error}")))?;
    let thumbnail = image.thumbnail(640, 640).to_rgb8();

    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 86)
        .encode_image(&thumbnail)
        .map_err(|error| AppError::Other(format!("cover encode error: {error}")))?;
    Ok(output)
}

/// Convert an accessor value into a non-empty owned string.
fn owned(value: Option<Cow<'_, str>>) -> Option<String> {
    value
        .map(|text| text.into_owned())
        .filter(|text| !text.is_empty())
}

fn probe_tagged(bytes: &[u8]) -> Result<lofty::file::TaggedFile, AppError> {
    let mut reader = std::io::Cursor::new(bytes);
    Probe::new(&mut reader)
        .guess_file_type()
        .map_err(|error| AppError::Other(format!("metadata io error: {error}")))?
        .read()
        .map_err(|error| AppError::Other(format!("metadata parse error: {error}")))
}

/// Parse a tagged file, repairing recoverable ID3v2 damage on failure.
///
/// `lofty` aborts the whole tag when a single frame cannot be decoded (for
/// example a text frame whose UTF-16 payload has an odd length). Left as-is,
/// one broken frame blocks every tag operation, including the editor. Retry
/// against a sanitized copy with the offending frames removed; the valid
/// frames are kept, and a subsequent write replaces the damaged tag.
fn parse_tagged(bytes: &[u8]) -> Result<lofty::file::TaggedFile, AppError> {
    match probe_tagged(bytes) {
        Ok(tagged) => Ok(tagged),
        Err(error) => {
            // A file can contain more than one ID3v2 tag (front tag, RIFF
            // chunks), so repair until the parse succeeds or nothing changes.
            let mut repaired = bytes.to_vec();
            for _ in 0..4 {
                let Some(next) = repair_id3v2(&repaired) else {
                    break;
                };
                match probe_tagged(&next) {
                    Ok(tagged) => return Ok(tagged),
                    Err(_) => repaired = next,
                }
            }
            Err(error)
        }
    }
}

/// An ID3v2 tag with undecodable frames removed.
struct Id3v2Repair {
    /// Replacement bytes for the old tag (header + surviving frames).
    bytes: Vec<u8>,
    /// Length of the old tag in the original buffer, including header/footer.
    old_len: usize,
}

/// Decode a four-byte syncsafe integer.
fn synchsafe_to_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 4 || bytes.iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    Some(
        (u32::from(bytes[0]) << 21)
            | (u32::from(bytes[1]) << 14)
            | (u32::from(bytes[2]) << 7)
            | u32::from(bytes[3]),
    )
}

/// Encode a four-byte syncsafe integer.
fn u32_to_synchsafe(value: u32) -> Option<[u8; 4]> {
    (value <= 0x0FFF_FFFF).then_some([
        ((value >> 21) & 0x7F) as u8,
        ((value >> 14) & 0x7F) as u8,
        ((value >> 7) & 0x7F) as u8,
        (value & 0x7F) as u8,
    ])
}

/// Whether a frame that begins with a text-encoding byte is undecodable.
///
/// Only the fields `lofty` reads as non-terminated text are checked: those are
/// where an odd UTF-16 payload (or a missing/incorrect BOM) aborts parsing.
fn text_field_is_malformed(encoding: u8, bytes: &[u8], require_bom: bool) -> bool {
    match encoding {
        1 => {
            if bytes.is_empty() {
                return false;
            }
            if !bytes.len().is_multiple_of(2) {
                return true;
            }
            if !require_bom {
                return false;
            }
            !(bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]))
        }
        // UTF-16BE has no byte order mark.
        2 => !bytes.len().is_multiple_of(2),
        3 => std::str::from_utf8(bytes).is_err(),
        _ => false,
    }
}

/// Skip a field terminated by a null (or UTF-16 null pair) and return the rest.
fn skip_encoded_terminator(bytes: &[u8], encoding: u8) -> &[u8] {
    match encoding {
        1 | 2 => {
            let mut index = 0;
            while index + 1 < bytes.len() {
                if bytes[index] == 0 && bytes[index + 1] == 0 {
                    return &bytes[index + 2..];
                }
                index += 2;
            }
            &[]
        }
        _ => match bytes.iter().position(|&byte| byte == 0) {
            Some(index) => &bytes[index + 1..],
            None => &[],
        },
    }
}

/// Whether a frame's payload would make `lofty` reject the whole tag.
fn frame_is_malformed(id: &[u8], content: &[u8], major: u8) -> bool {
    let Ok(id) = std::str::from_utf8(id) else {
        return false;
    };

    // Frames whose payload starts with an encoding byte.
    let encoding_first = if major == 2 {
        id.starts_with('T')
            || matches!(
                id,
                "COM" | "ULT" | "PIC" | "WXX" | "GEO" | "SLT" | "OWE" | "USR"
            )
    } else {
        id.starts_with('T')
            || matches!(
                id,
                "COMM" | "USLT" | "SYLT" | "APIC" | "GEOB" | "OWNE" | "USER" | "WXXX"
            )
    };
    if !encoding_first {
        return false;
    }

    let Some((&encoding, rest)) = content.split_first() else {
        return false;
    };
    let max_encoding = if major >= 3 { 3 } else { 1 };
    if encoding > max_encoding {
        return true;
    }
    if encoding == 0 {
        return false;
    }

    match id {
        // Encoding + language + terminated description + text content.
        "COMM" | "USLT" | "USR" => {
            let Some(after_language) = rest.get(3..) else {
                return false;
            };
            let content = skip_encoded_terminator(after_language, encoding);
            text_field_is_malformed(encoding, content, true)
        }
        // Encoding + terminated price + 8-byte date + seller text.
        "OWNE" | "OWE" => {
            let Some(after_price) = rest
                .iter()
                .position(|&byte| byte == 0)
                .and_then(|index| rest.get(index + 1..))
            else {
                return false;
            };
            let Some(seller) = after_price.get(8..) else {
                return false;
            };
            text_field_is_malformed(encoding, seller, true)
        }
        // Encoding + terminated description + text content. The content may
        // inherit the description's BOM, so only the payload length matters.
        "TXXX" | "TXX" => {
            let content = skip_encoded_terminator(rest, encoding);
            text_field_is_malformed(encoding, content, false)
        }
        // Remaining encoding-first frames use terminated strings or binary
        // payloads; they cannot hit the odd-length text decoder.
        "APIC" | "PIC" | "GEOB" | "GEO" | "SYLT" | "SLT" | "WXXX" | "WXX" => false,
        // Plain text and timestamp frames: the payload is the text itself.
        _ => text_field_is_malformed(encoding, rest, true),
    }
}

/// Rebuild one ID3v2 tag without frames that `lofty` cannot decode.
fn repair_id3v2_tag(buffer: &[u8], start: usize) -> Option<Id3v2Repair> {
    let header = buffer.get(start..start.checked_add(10)?)?;
    if &header[0..3] != b"ID3" {
        return None;
    }

    let major = header[3];
    if !(2..=4).contains(&major) {
        return None;
    }
    let flags = header[5];
    // Unsynchronised and extended-header tags need their own parsing rules;
    // skipping the repair keeps their bytes untouched.
    if flags & 0x80 != 0 || (major >= 3 && flags & 0x40 != 0) {
        return None;
    }
    if major == 2 && flags & 0x40 != 0 {
        return None;
    }

    let size = usize::try_from(synchsafe_to_u32(&header[6..10])?).ok()?;
    let payload_start = start.checked_add(10)?;
    let payload_end = payload_start.checked_add(size)?;
    if payload_end > buffer.len() {
        return None;
    }

    let has_footer = major >= 3 && flags & 0x10 != 0;
    let old_len = (payload_end - start).checked_add(if has_footer { 10 } else { 0 })?;
    if start.checked_add(old_len)? > buffer.len() {
        return None;
    }

    let frame_header_len = if major == 2 { 6 } else { 10 };
    let id_len = if major == 2 { 3 } else { 4 };
    let mut position = payload_start;
    let mut kept = Vec::new();
    let mut removed = false;

    while position + frame_header_len <= payload_end {
        // A zero byte starts the padding.
        if buffer[position] == 0 {
            break;
        }

        let id = &buffer[position..position + id_len];
        if !id.iter().all(u8::is_ascii_alphanumeric) {
            // The frame stream is corrupt; drop the remainder as padding.
            removed = true;
            break;
        }

        let frame_size = match major {
            2 => {
                (usize::from(buffer[position + 3]) << 16)
                    | (usize::from(buffer[position + 4]) << 8)
                    | usize::from(buffer[position + 5])
            }
            3 => usize::try_from(u32::from_be_bytes([
                buffer[position + 4],
                buffer[position + 5],
                buffer[position + 6],
                buffer[position + 7],
            ]))
            .ok()?,
            _ => usize::try_from(synchsafe_to_u32(&buffer[position + 4..position + 8])?).ok()?,
        };

        let content_start = position + frame_header_len;
        let Some(content_end) = content_start.checked_add(frame_size) else {
            removed = true;
            break;
        };
        if frame_size == 0 || content_end > payload_end {
            removed = true;
            if frame_size == 0 {
                // Empty frames are skipped by `lofty`; keep scanning the
                // remaining frames instead of discarding them.
                position = content_start;
                continue;
            }
            break;
        }

        let frame_flags = if major >= 3 {
            u16::from_be_bytes([buffer[position + 8], buffer[position + 9]])
        } else {
            0
        };
        let content = &buffer[content_start..content_end];

        // Compressed/encrypted/grouped frames have extra leading bytes, so
        // their text cannot be validated here. Keep them untouched.
        if frame_flags == 0 && frame_is_malformed(id, content, major) {
            removed = true;
        } else {
            kept.extend_from_slice(&buffer[position..content_end]);
        }

        position = content_end;
    }

    if !removed {
        return None;
    }

    let new_size = u32_to_synchsafe(u32::try_from(kept.len()).ok()?)?;
    let mut bytes = Vec::with_capacity(10 + kept.len());
    bytes.extend_from_slice(b"ID3");
    bytes.push(major);
    bytes.push(header[4]);
    // Drop the footer/extended-header flags along with those bytes.
    bytes.push(flags & !0x10);
    bytes.extend_from_slice(&new_size);
    bytes.extend_from_slice(&kept);

    Some(Id3v2Repair { bytes, old_len })
}

/// Sanitize every top-level ID3v2 tag in an audio buffer.
fn repair_id3v2(buffer: &[u8]) -> Option<Vec<u8>> {
    if buffer.starts_with(b"ID3") {
        let repair = repair_id3v2_tag(buffer, 0)?;
        let mut output = Vec::with_capacity(buffer.len());
        output.extend_from_slice(&repair.bytes);
        output.extend_from_slice(&buffer[repair.old_len..]);
        return Some(output);
    }

    // WAV (RIFF) and AIFF (FORM) store ID3v2 in a chunk instead.
    let little_endian = match buffer.get(0..4)? {
        b"RIFF" => true,
        b"FORM" => false,
        _ => return None,
    };
    if buffer.len() < 12 {
        return None;
    }
    let read_u32 = |bytes: &[u8]| {
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };

    let mut output = buffer.to_vec();
    let mut position = 12;
    let mut changed = false;
    while position + 8 <= output.len() {
        let chunk_id = [
            output[position],
            output[position + 1],
            output[position + 2],
            output[position + 3],
        ];
        let chunk_size = usize::try_from(read_u32(&output[position + 4..position + 8])).ok()?;
        let data_start = position + 8;
        let Some(data_end) = data_start.checked_add(chunk_size) else {
            break;
        };
        if data_end > output.len() {
            break;
        }

        if &chunk_id == b"ID3 " || &chunk_id == b"id3 " {
            if let Some(repair) = repair_id3v2_tag(&output, data_start) {
                let old_padded = repair.old_len + repair.old_len % 2;
                let Some(old_end) = data_start.checked_add(old_padded) else {
                    break;
                };
                if old_end > output.len() {
                    break;
                }

                let mut replacement = repair.bytes;
                let new_size = replacement.len();
                if new_size % 2 != 0 {
                    replacement.push(0);
                }
                output.splice(data_start..old_end, replacement);

                output[position + 4..position + 8].copy_from_slice(
                    &(if little_endian {
                        (new_size as u32).to_le_bytes()
                    } else {
                        (new_size as u32).to_be_bytes()
                    }),
                );
                let stream_size = u32::try_from(output.len().checked_sub(8)?).ok()?;
                output[4..8].copy_from_slice(
                    &(if little_endian {
                        stream_size.to_le_bytes()
                    } else {
                        stream_size.to_be_bytes()
                    }),
                );

                changed = true;
                position = data_start + new_size + new_size % 2;
                continue;
            }
        }

        position = data_end + chunk_size % 2;
    }

    changed.then_some(output)
}

/// Read every editable tag from an audio buffer.
pub fn read_tags(bytes: &[u8]) -> Result<TrackTags, AppError> {
    let tagged = parse_tagged(bytes)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let album_artist = tag
        .and_then(|tag| tag.get_string(&ItemKey::AlbumArtist))
        .map(str::to_string)
        .filter(|value| !value.is_empty());

    Ok(TrackTags {
        title: owned(tag.and_then(|tag| tag.title())),
        artist: owned(tag.and_then(|tag| tag.artist())),
        album: owned(tag.and_then(|tag| tag.album())),
        album_artist,
        genre: owned(tag.and_then(|tag| tag.genre())),
        comment: owned(tag.and_then(|tag| tag.comment())),
        year: tag.and_then(|tag| tag.year()),
        track_number: tag.and_then(|tag| tag.track()),
        track_total: tag.and_then(|tag| tag.track_total()),
        disc_number: tag.and_then(|tag| tag.disk()),
        disc_total: tag.and_then(|tag| tag.disk_total()),
        has_cover: tag.map(|tag| !tag.pictures().is_empty()).unwrap_or(false),
    })
}

/// Apply a text field: set when non-empty, remove when cleared, leave when absent.
fn set_text<Set, Clear>(tag: &mut Tag, value: &Option<String>, set: Set, clear: Clear)
where
    Set: FnOnce(&mut Tag, String),
    Clear: FnOnce(&mut Tag),
{
    match value.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => set(tag, text.to_string()),
        Some(_) => clear(tag),
        None => {}
    }
}

/// Rewrite tags in an audio buffer, returning the new file bytes.
///
/// `cover` replaces the front cover when present; `remove_cover` strips all
/// embedded art otherwise. When neither is set the existing artwork is kept.
pub fn write_tags(
    bytes: &[u8],
    tags: &TrackTags,
    cover: Option<(MimeType, Vec<u8>)>,
    remove_cover: bool,
) -> Result<Vec<u8>, AppError> {
    let mut tagged = parse_tagged(bytes)?;

    // Write into the file's native tag type, creating it when the file has none.
    let tag_type = tagged.primary_tag_type();
    if !tagged.contains_tag_type(tag_type) {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .tag_mut(tag_type)
        .ok_or_else(|| AppError::Other("该音频格式不支持写入标签".to_string()))?;

    set_text(
        tag,
        &tags.title,
        |tag, value| tag.set_title(value),
        |tag| tag.remove_title(),
    );
    set_text(
        tag,
        &tags.artist,
        |tag, value| tag.set_artist(value),
        |tag| tag.remove_artist(),
    );
    set_text(
        tag,
        &tags.album,
        |tag, value| tag.set_album(value),
        |tag| tag.remove_album(),
    );
    set_text(
        tag,
        &tags.genre,
        |tag, value| tag.set_genre(value),
        |tag| tag.remove_genre(),
    );
    set_text(
        tag,
        &tags.comment,
        |tag, value| tag.set_comment(value),
        |tag| tag.remove_comment(),
    );

    match tags.album_artist.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => {
            tag.insert_text(ItemKey::AlbumArtist, text.to_string());
        }
        Some(_) => {
            tag.remove_key(&ItemKey::AlbumArtist);
        }
        None => {}
    }

    match tags.year {
        Some(value) => tag.set_year(value),
        None => tag.remove_year(),
    }
    match tags.track_number {
        Some(value) => tag.set_track(value),
        None => tag.remove_track(),
    }
    match tags.track_total {
        Some(value) => tag.set_track_total(value),
        None => tag.remove_track_total(),
    }
    match tags.disc_number {
        Some(value) => tag.set_disk(value),
        None => tag.remove_disk(),
    }
    match tags.disc_total {
        Some(value) => tag.set_disk_total(value),
        None => tag.remove_disk_total(),
    }

    if let Some((mime, data)) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(mime),
            None,
            data,
        ));
    } else if remove_cover {
        while !tag.pictures().is_empty() {
            tag.remove_picture(0);
        }
    }

    let mut output = std::io::Cursor::new(bytes.to_vec());
    // Drop the legacy ID3v1 tag so `lofty` never attempts to write its
    // fixed-width fields (its `resize_string` panics on multi-byte chars).
    // ID3v2 keeps the full text and is what modern players read.
    let _ = tagged.remove(lofty::tag::TagType::Id3v1);
    tagged
        .save_to(&mut output, WriteOptions::default())
        .map_err(|error| AppError::Other(format!("metadata write error: {error}")))?;
    Ok(output.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id3v24_frame(id: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        frame.extend_from_slice(id);
        frame.extend_from_slice(&u32_to_synchsafe(content.len() as u32).unwrap());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(content);
        frame
    }

    fn id3v24_tag(frames: &[Vec<u8>]) -> Vec<u8> {
        let payload: Vec<u8> = frames.concat();
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3\x04\x00\x00");
        tag.extend_from_slice(&u32_to_synchsafe(payload.len() as u32).unwrap());
        tag.extend_from_slice(&payload);
        tag
    }

    fn id3v23_tag(frames: &[Vec<u8>]) -> Vec<u8> {
        let payload: Vec<u8> = frames.concat();
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3\x03\x00\x00");
        tag.extend_from_slice(&u32_to_synchsafe(payload.len() as u32).unwrap());
        tag.extend_from_slice(&payload);
        tag
    }

    fn id3v23_frame(id: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        frame.extend_from_slice(id);
        frame.extend_from_slice(&(content.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(content);
        frame
    }

    fn wav_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(id);
        chunk.extend_from_slice(&(data.len() as u32).to_le_bytes());
        chunk.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            chunk.push(0);
        }
        chunk
    }

    fn wav_with_id3(tag: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
        fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
        fmt.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
        fmt.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
        fmt.extend_from_slice(&2u16.to_le_bytes()); // block align
        fmt.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(&wav_chunk(b"fmt ", &fmt));
        body.extend_from_slice(&wav_chunk(b"id3 ", tag));
        body.extend_from_slice(&wav_chunk(b"data", &[0, 0]));

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(body.len() as u32).to_le_bytes());
        wav.extend_from_slice(&body);
        wav
    }

    #[test]
    fn reads_tags_past_an_odd_length_utf16_frame() {
        let tag = id3v24_tag(&[
            id3v24_frame(b"TIT2", &[3, b'H', b'e', b'l', b'l', b'o']),
            // UTF-16 payload with an odd number of bytes.
            id3v24_frame(b"TPE1", &[1, 0xFF, 0xFE, 0x00, b'A', 0x00]),
        ]);
        let wav = wav_with_id3(&tag);

        let tags = read_tags(&wav).expect("a broken frame must not fail the read");
        assert_eq!(tags.title.as_deref(), Some("Hello"));
        assert_eq!(tags.artist, None);
    }

    #[test]
    fn reads_tags_past_an_odd_length_utf16_frame_v23() {
        let tag = id3v23_tag(&[
            id3v23_frame(b"TIT2", &[0, b'H', b'e', b'l', b'l', b'o']),
            id3v23_frame(b"TPE1", &[1, 0xFF, 0xFE, 0x00, b'A', 0x00]),
        ]);
        let wav = wav_with_id3(&tag);

        let tags = read_tags(&wav).expect("a broken frame must not fail the read");
        assert_eq!(tags.title.as_deref(), Some("Hello"));
        assert_eq!(tags.artist, None);
    }

    #[test]
    fn writing_tags_replaces_the_damaged_tag() {
        let tag = id3v24_tag(&[
            id3v24_frame(b"TIT2", &[3, b'H', b'e', b'l', b'l', b'o']),
            id3v24_frame(b"TPE1", &[1, 0xFF, 0xFE, 0x00, b'A', 0x00]),
        ]);
        let wav = wav_with_id3(&tag);

        let tags = read_tags(&wav).expect("read");
        let mut edited = tags;
        edited.title = Some("Renamed".to_string());
        let written = write_tags(&wav, &edited, None, false).expect("write");

        let reread = read_tags(&written).expect("reread");
        assert_eq!(reread.title.as_deref(), Some("Renamed"));
        assert_eq!(reread.artist, None);
    }

    #[test]
    fn repairs_a_front_id3v24_tag() {
        let tag = id3v24_tag(&[
            id3v24_frame(b"TIT2", &[3, b'H', b'e', b'l', b'l', b'o']),
            id3v24_frame(b"TPE1", &[1, 0xFF, 0xFE, 0x00, b'A', 0x00]),
        ]);
        let repaired = repair_id3v2(&tag).expect("repair");
        let wav = wav_with_id3(&repaired);
        let tags = read_tags(&wav).expect("parse repaired tag");
        assert_eq!(tags.title.as_deref(), Some("Hello"));
        assert_eq!(tags.artist, None);
    }

    #[test]
    fn leaves_well_formed_tags_untouched() {
        let tag = id3v24_tag(&[id3v24_frame(b"TIT2", &[3, b'H', b'e', b'l', b'l', b'o'])]);
        assert!(repair_id3v2(&tag).is_none());
    }

    #[test]
    fn encodes_thumbnail_as_jpeg() {
        let mut png = Vec::new();
        let image = image::RgbImage::from_fn(4, 4, |x, y| {
            image::Rgb([(x * 60) as u8, (y * 60) as u8, 200])
        });
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode png");

        let jpeg = encode_thumbnail(&png).expect("thumbnail");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]), "expected a JPEG header");
    }
}
