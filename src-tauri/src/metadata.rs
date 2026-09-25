//! Audio metadata extraction.
//!
//! Parsing happens on an in-memory buffer filled from a single HTTP `Range`
//! request (see `WebDavClient::get_range`), so metadata — including embedded
//! cover art — is read from the file head without downloading the whole file.

use std::time::Duration;

use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

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
