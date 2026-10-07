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
    let mut reader = std::io::Cursor::new(bytes);
    let tagged = Probe::new(&mut reader)
        .guess_file_type()
        .map_err(|error| AppError::Other(format!("metadata io error: {error}")))?
        .read()
        .map_err(|error| AppError::Other(format!("metadata parse error: {error}")))?;

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

fn parse_tagged(bytes: &[u8]) -> Result<lofty::file::TaggedFile, AppError> {
    let mut reader = std::io::Cursor::new(bytes);
    Probe::new(&mut reader)
        .guess_file_type()
        .map_err(|error| AppError::Other(format!("metadata io error: {error}")))?
        .read()
        .map_err(|error| AppError::Other(format!("metadata parse error: {error}")))
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

    set_text(tag, &tags.title, |tag, value| tag.set_title(value), |tag| {
        tag.remove_title()
    });
    set_text(tag, &tags.artist, |tag, value| tag.set_artist(value), |tag| {
        tag.remove_artist()
    });
    set_text(tag, &tags.album, |tag, value| tag.set_album(value), |tag| {
        tag.remove_album()
    });
    set_text(tag, &tags.genre, |tag, value| tag.set_genre(value), |tag| {
        tag.remove_genre()
    });
    set_text(tag, &tags.comment, |tag, value| tag.set_comment(value), |tag| {
        tag.remove_comment()
    });

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
    tagged
        .save_to(&mut output, WriteOptions::default())
        .map_err(|error| AppError::Other(format!("metadata write error: {error}")))?;
    Ok(output.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

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
