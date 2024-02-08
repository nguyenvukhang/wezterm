//! Images.
//! This module has some helpers for modeling terminal cells that are filled
//! with image data.
//! We're targeting the iTerm image protocol initially, with sixel as an obvious
//! follow up.
//! Kitty has an extensive and complex graphics protocol
//! whose docs are here:
//! <https://github.com/kovidgoyal/kitty/blob/master/docs/graphics-protocol.rst>
//! Both iTerm2 and Sixel appear to have semantics that allow replacing the
//! contents of a single chararcter cell with image data, whereas the kitty
//! protocol appears to track the images out of band as attachments with
//! z-order.

use crate::error::InternalError;
use ordered_float::NotNan;
#[cfg(feature = "use_serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::hash::Hash;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

#[cfg(feature = "use_serde")]
fn deserialize_notnan<'de, D>(deserializer: D) -> Result<NotNan<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f32::deserialize(deserializer)?;
    NotNan::new(value).map_err(|e| serde::de::Error::custom(format!("{:?}", e)))
}

#[cfg(feature = "use_serde")]
#[cfg_attr(feature = "cargo-clippy", allow(clippy::trivially_copy_pass_by_ref))]
fn serialize_notnan<S>(value: &NotNan<f32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.into_inner().serialize(serializer)
}

#[cfg_attr(feature = "use_serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureCoordinate {
    #[cfg_attr(
        feature = "use_serde",
        serde(
            deserialize_with = "deserialize_notnan",
            serialize_with = "serialize_notnan"
        )
    )]
    pub x: NotNan<f32>,
    #[cfg_attr(
        feature = "use_serde",
        serde(
            deserialize_with = "deserialize_notnan",
            serialize_with = "serialize_notnan"
        )
    )]
    pub y: NotNan<f32>,
}

impl TextureCoordinate {
    pub fn new(x: NotNan<f32>, y: NotNan<f32>) -> Self {
        Self { x, y }
    }

    pub fn new_f32(x: f32, y: f32) -> Self {
        let x = NotNan::new(x).unwrap();
        let y = NotNan::new(y).unwrap();
        Self::new(x, y)
    }
}

#[cfg_attr(feature = "use_serde", derive(Serialize, Deserialize))]
#[derive(Clone, PartialEq, Eq)]
pub enum ImageDataType {
    /// Data is RGBA u8 data
    Rgba8 {
        data: Vec<u8>,
        width: u32,
        height: u32,
        hash: [u8; 32],
    },
    /// Data is an animated sequence
    AnimRgba8 {
        width: u32,
        height: u32,
        durations: Vec<Duration>,
        frames: Vec<Vec<u8>>,
        hashes: Vec<[u8; 32]>,
    },
}

impl std::fmt::Debug for ImageDataType {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Rgba8 {
                data,
                width,
                height,
                hash,
            } => fmt
                .debug_struct("Rgba8")
                .field("data_of_len", &data.len())
                .field("width", &width)
                .field("height", &height)
                .field("hash", &hash)
                .finish(),
            Self::AnimRgba8 {
                frames,
                width,
                height,
                durations,
                hashes,
            } => fmt
                .debug_struct("AnimRgba8")
                .field("frames_of_len", &frames.len())
                .field("width", &width)
                .field("height", &height)
                .field("durations", durations)
                .field("hashes", hashes)
                .finish(),
        }
    }
}

impl ImageDataType {
    pub fn new_single_frame(width: u32, height: u32, data: Vec<u8>) -> Self {
        let hash = Self::hash_bytes(&data);
        assert_eq!(
            width * height * 4,
            data.len() as u32,
            "invalid dimensions {}x{} for pixel data of length {}",
            width,
            height,
            data.len()
        );
        Self::Rgba8 {
            width,
            height,
            data,
            hash,
        }
    }

    /// Black pixels
    pub fn placeholder() -> Self {
        let mut data = vec![];
        let size = 8;
        for _ in 0..size * size {
            data.extend_from_slice(&[0, 0, 0, 0xff]);
        }
        ImageDataType::new_single_frame(size, size, data)
    }

    pub fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        hasher.finalize().into()
    }

    pub fn compute_hash(&self) -> [u8; 32] {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        match self {
            ImageDataType::Rgba8 { data, .. } => hasher.update(data),
            ImageDataType::AnimRgba8 {
                frames, durations, ..
            } => {
                for data in frames {
                    hasher.update(data);
                }
                for d in durations {
                    let d = d.as_secs_f32();
                    let b = d.to_ne_bytes();
                    hasher.update(b);
                }
            }
        };
        hasher.finalize().into()
    }

    /// Divides the animation frame durations by the provided
    /// speed_factor, so a factor of 2 will halve the duration.
    /// # Panics
    /// if the speed_factor is negative, non-finite or the result
    /// overflows the allow Duration range.
    pub fn adjust_speed(&mut self, speed_factor: f32) {
        match self {
            Self::AnimRgba8 { durations, .. } => {
                for d in durations {
                    *d = d.mul_f32(1. / speed_factor);
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "use_image")]
    pub fn dimensions(&self) -> Result<(u32, u32), InternalError> {
        match self {
            ImageDataType::AnimRgba8 { width, height, .. }
            | ImageDataType::Rgba8 { width, height, .. } => Ok((*width, *height)),
        }
    }

    /// Migrate an in-memory encoded image blob to on-disk to reduce
    /// the memory footprint
    pub fn swap_out(self) -> Result<Self, InternalError> {
        Ok(self)
    }

    pub fn decode(self) -> Self {
        self
    }
}

#[cfg_attr(feature = "use_serde", derive(Serialize, Deserialize))]
pub struct ImageData {
    data: Mutex<ImageDataType>,
    hash: [u8; 32],
}

struct HexSlice<'a>(&'a [u8]);
impl<'a> std::fmt::Display for HexSlice<'a> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        for byte in self.0 {
            write!(fmt, "{byte:x}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("ImageData")
            .field("data", &self.data)
            .field("hash", &format_args!("{}", HexSlice(&self.hash)))
            .finish()
    }
}

impl Eq for ImageData {}
impl PartialEq for ImageData {
    fn eq(&self, rhs: &Self) -> bool {
        self.hash == rhs.hash
    }
}

impl ImageData {
    pub fn with_data(data: ImageDataType) -> Self {
        let hash = data.compute_hash();
        Self {
            data: Mutex::new(data),
            hash,
        }
    }

    /// Returns the in-memory footprint
    pub fn len(&self) -> usize {
        match &*self.data() {
            ImageDataType::Rgba8 { data, .. } => data.len(),
            ImageDataType::AnimRgba8 { frames, .. } => frames.len() * frames[0].len(),
        }
    }

    pub fn data(&self) -> MutexGuard<ImageDataType> {
        self.data.lock().unwrap()
    }

    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
}
