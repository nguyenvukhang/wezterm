use crate::escape::osc::{base64_decode, base64_encode};
use std::collections::BTreeMap;
use std::fmt::{Display, Error as FmtError, Formatter};
use std::io::{Read, Seek};

fn get<'a>(keys: &BTreeMap<&str, &'a str>, k: &str) -> Option<&'a str> {
    keys.get(k).map(|&s| s)
}

fn geti<T: std::str::FromStr>(keys: &BTreeMap<&str, &str>, k: &str) -> Option<T> {
    get(keys, k).and_then(|s| s.parse().ok())
}

fn set<T: std::string::ToString>(
    keys: &mut BTreeMap<&'static str, String>,
    k: &'static str,
    v: &Option<T>,
) {
    if let Some(v) = v {
        keys.insert(k, v.to_string());
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum KittyImageData {
    /// The data bytes, baes64-encoded fragments.
    /// t='d'
    Direct(String),
    DirectBin(Vec<u8>),
    /// The path to a file containing the data.
    /// t='f'
    File {
        path: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },
    /// The path to a temporary file containing the data.
    /// If the path is in a known temporary location,
    /// it should be removed once the data has been read
    /// t='t'
    TemporaryFile {
        path: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },

    /// The name of a shared memory object.
    /// Can be opened via shm_open() and then should be removed
    /// via shm_unlink().
    /// On Windows, OpenFileMapping(), MapViewOfFile(), UnmapViewOfFile()
    /// and CloseHandle() are used to access and release the data.
    /// t='s'
    SharedMem {
        name: String,
        /// the amount of data to read.
        /// S=...
        data_size: Option<u32>,
        /// The offset at which to read.
        /// O=...
        data_offset: Option<u32>,
    },
}

impl std::fmt::Debug for KittyImageData {
    fn fmt(&self, fmt: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::Direct(data) => write!(fmt, "Direct({} bytes of data)", data.len()),
            Self::DirectBin(data) => write!(fmt, "DirectBin({} bytes of data)", data.len()),
            Self::File {
                path,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("File")
                .field("path", &path)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
            Self::TemporaryFile {
                path,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("TemporaryFile")
                .field("path", &path)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
            Self::SharedMem {
                name,
                data_offset,
                data_size,
            } => fmt
                .debug_struct("SharedMem")
                .field("name", &name)
                .field("data_offset", &data_offset)
                .field("data_size", data_size)
                .finish(),
        }
    }
}

impl KittyImageData {
    /// Take the image data bytes.
    /// This operation is not repeatable as some of the sources require
    /// removing the underlying file or shared memory object as part
    /// of the read operaiton.
    pub fn load_data(self) -> std::io::Result<Vec<u8>> {
        fn read_from_file(
            path: &str,
            data_offset: Option<u32>,
            data_size: Option<u32>,
        ) -> std::io::Result<Vec<u8>> {
            let mut f = std::fs::File::open(path)?;
            if let Some(offset) = data_offset {
                f.seek(std::io::SeekFrom::Start(offset.into()))?;
            }
            if let Some(len) = data_size {
                let mut res = vec![0u8; len as usize];
                f.read_exact(&mut res)?;
                Ok(res)
            } else {
                let mut res = vec![];
                f.read_to_end(&mut res)?;
                Ok(res)
            }
        }

        match self {
            Self::Direct(data) => base64_decode(data).or_else(|err| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("base64 decode: {err:#}"),
                ))
            }),
            Self::DirectBin(bin) => Ok(bin),
            Self::File {
                path,
                data_offset,
                data_size,
            } => read_from_file(&path, data_offset, data_size),
            Self::TemporaryFile {
                path,
                data_offset,
                data_size,
            } => {
                let data = read_from_file(&path, data_offset, data_size)?;
                // need to sanity check that the path looks like a reasonable
                // temporary directory path before blindly unlinking it here.

                fn looks_like_temp_path(p: &str) -> bool {
                    if p.starts_with("/tmp/")
                        || p.starts_with("/var/tmp/")
                        || p.starts_with("/dev/shm/")
                    {
                        return true;
                    }

                    if let Ok(t) = std::env::var("TMPDIR") {
                        if p.starts_with(&t) {
                            return true;
                        }
                    }

                    false
                }

                if looks_like_temp_path(&path) {
                    if let Err(err) = std::fs::remove_file(&path) {
                        log::error!(
                            "Unable to remove kitty image protocol temporary file {}: {:#}",
                            path,
                            err
                        );
                    }
                } else {
                    log::warn!(
                        "kitty image protocol temporary file {} isn't in a known \
                                temporary directory; won't try to remove it",
                        path
                    );
                }

                Ok(data)
            }
            Self::SharedMem {
                name,
                data_offset,
                data_size,
            } => read_shared_memory_data(&name, data_offset, data_size),
        }
    }
}

#[cfg(all(unix, not(target_os = "android")))]
fn read_shared_memory_data(
    name: &str,
    data_offset: Option<u32>,
    data_size: Option<u32>,
) -> std::result::Result<std::vec::Vec<u8>, std::io::Error> {
    use nix::sys::mman::{shm_open, shm_unlink};
    use std::fs::File;
    use std::os::unix::io::FromRawFd;

    let raw_fd = shm_open(
        name,
        nix::fcntl::OFlag::O_RDONLY,
        nix::sys::stat::Mode::empty(),
    )
    .map_err(|_| {
        let err = std::io::Error::last_os_error();
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("shm_open {} failed: {:#}", name, err),
        )
    })?;
    let mut f = unsafe { File::from_raw_fd(raw_fd) };
    if let Some(offset) = data_offset {
        f.seek(std::io::SeekFrom::Start(offset.into()))?;
    }
    let data = if let Some(len) = data_size {
        let mut res = vec![0u8; len as usize];
        f.read_exact(&mut res)?;
        res
    } else {
        let mut res = vec![];
        f.read_to_end(&mut res)?;
        res
    };

    if let Err(err) = shm_unlink(name) {
        log::warn!(
            "Unable to unlink kitty image protocol shm file {}: {:#}",
            name,
            err
        );
    }
    Ok(data)
}

#[cfg(all(unix, target_os = "android"))]
fn read_shared_memory_data(
    _name: &str,
    _data_offset: Option<u32>,
    _data_size: Option<u32>,
) -> std::result::Result<std::vec::Vec<u8>, std::io::Error> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(windows)]
mod win {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::memoryapi::{
        MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_ALL_ACCESS,
    };
    use winapi::um::winnt::{HANDLE, MEMORY_BASIC_INFORMATION};

    struct HandleWrapper {
        handle: HANDLE,
    }

    struct SharedMemObject {
        _handle: HandleWrapper,
        buf: *mut u8,
    }

    impl Drop for HandleWrapper {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    impl Drop for SharedMemObject {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(self.buf as _);
            }
        }
    }

    /// Convert a rust string to a windows wide string
    fn wide_string(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn read_shared_memory_data(
        name: &str,
        data_offset: Option<u32>,
        data_size: Option<u32>,
    ) -> std::result::Result<std::vec::Vec<u8>, std::io::Error> {
        let wide_name = wide_string(&name);

        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, wide_name.as_ptr()) };
        if handle.is_null() {
            let err = std::io::Error::last_os_error();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("OpenFileMappingW {} failed: {:#}", name, err),
            ));
        }

        let handle_wrapper = HandleWrapper { handle };
        let buf = unsafe { MapViewOfFile(handle_wrapper.handle, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
        if buf.is_null() {
            let err = std::io::Error::last_os_error();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("MapViewOfFile failed: {:#}", err),
            ));
        }

        let shm = SharedMemObject {
            _handle: handle_wrapper,
            buf: buf as *mut u8,
        };

        let mut memory_info = MEMORY_BASIC_INFORMATION::default();
        let res = unsafe {
            VirtualQuery(
                shm.buf as _,
                &mut memory_info as *mut MEMORY_BASIC_INFORMATION,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if res == 0 {
            let err = std::io::Error::last_os_error();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "Can't get the size of Shared Memory, VirtualQuery failed: {:#}",
                    err
                ),
            ));
        }
        let mut size = memory_info.RegionSize;
        let offset = data_offset.unwrap_or(0) as usize;
        if offset >= size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "offset {} bigger than or equal to shm region size {}",
                    offset, size
                ),
            ));
        }
        size = size.saturating_sub(offset);
        if let Some(val) = data_size {
            size = size.min(val as usize);
        }
        let buf_slice = unsafe { std::slice::from_raw_parts(shm.buf.add(offset), size) };
        let data = buf_slice.to_vec();

        Ok(data)
    }
}

#[cfg(windows)]
use win::read_shared_memory_data;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KittyImageVerbosity {
    Verbose,
    OnlyErrors,
    Quiet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageFormat {
    /// f=24
    Rgb,
    /// f=32
    Rgba,
    /// f=100
    Png,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageCompression {
    None,
    /// o='z'
    Deflate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageTransmit {
    /// f=...
    pub format: Option<KittyImageFormat>,
    /// combination of t=... and d=...
    pub data: KittyImageData,
    /// s=...
    pub width: Option<u32>,
    /// v=...
    pub height: Option<u32>,
    /// The image id.
    /// i=...
    pub image_id: Option<u32>,
    /// The image number
    /// I=...
    pub image_number: Option<u32>,
    /// o=...
    pub compression: KittyImageCompression,

    /// m=0 or m=1
    pub more_data_follows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImagePlacement {
    /// source rectangle bounds.
    /// Default is whole image.
    /// x=...
    pub x: Option<u32>,
    pub y: Option<u32>,
    pub w: Option<u32>,
    pub h: Option<u32>,
    /// Place the image at an offset from the cell.
    /// X,Y must be <= cell metrics
    /// X=...
    pub x_offset: Option<u32>,
    /// Y=...
    pub y_offset: Option<u32>,
    /// Scale so that the image fits within this number of columns
    /// c=...
    pub columns: Option<u32>,
    /// Scale so that the image fits within this number of rows
    /// r=...
    pub rows: Option<u32>,
    /// By default, cursor will move to after the bottom right
    /// cell of the image placement.  do_not_move_cursor cursor
    /// set to true prevents that.
    /// C=0, C=1
    pub do_not_move_cursor: bool,
    /// Give an explicit placement id to this placement.
    /// p=...
    pub placement_id: Option<u32>,
    /// z=...
    pub z_index: Option<i32>,
}

/// When the uppercase form is used, the delete: field is set to true
/// which means that the underlying data is also released.  Otherwise,
/// the data is available to be placed again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KittyImageDelete {
    /// d='a' or d='A'.
    /// Delete all placements on visible screen
    All { delete: bool },
    /// d='i' or d='I'
    /// Delete all images with specified image_id.
    /// If placement_id is specified, then both image_id
    /// and placement_id must match
    ByImageId {
        image_id: u32,
        placement_id: Option<u32>,
        delete: bool,
    },
    /// d='n' or d='N'
    /// Delete newest image with specified image number.
    /// If placement_id is specified, then placement_id
    /// must also match.
    ByImageNumber {
        image_number: u32,
        placement_id: Option<u32>,
        delete: bool,
    },

    /// d='c' or d='C'
    /// Delete all placements that intersect with the current
    /// cursor position.
    AtCursorPosition { delete: bool },

    /// d='f' or d='F'
    /// Delete animation frames
    AnimationFrames { delete: bool },

    /// d='p' or d='P'
    /// Delete all placements that intersect the specified
    /// cell x and y coordinates
    DeleteAt { x: u32, y: u32, delete: bool },

    /// d='q' or d='Q'
    /// Delete all placements that intersect the specified
    /// cell x and y coordinates, with the specified z-index
    DeleteAtZ {
        x: u32,
        y: u32,
        z: i32,
        delete: bool,
    },

    /// d='x' or d='X'
    /// Delete all placements that intersect the specified column.
    DeleteColumn { x: u32, delete: bool },

    /// d='y' or d='Y'
    /// Delete all placements that intersect the specified row.
    DeleteRow { y: u32, delete: bool },

    /// d='z' or d='Z'
    /// Delete all placements that have the specified z-index.
    DeleteZ { z: i32, delete: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyFrameCompositionMode {
    AlphaBlending,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageFrameCompose {
    /// i=...
    pub image_id: Option<u32>,
    /// I=...
    pub image_number: Option<u32>,

    /// 1-based number of the frame which should be the base
    /// data for the new frame being created.
    /// If omitted, use background_pixel to specify color.
    /// c=...
    pub target_frame: Option<u32>,

    /// 1-based number of the frame which should be edited.
    /// If omitted, a new frame is created.
    /// r=...
    pub source_frame: Option<u32>,

    /// Left edge in pixels to update
    /// x=...
    pub x: Option<u32>,
    /// Top edge in pixels to update
    /// y=...
    pub y: Option<u32>,

    /// Width (in pixels) of the source and destination rectangles.
    /// By default the full width is used.
    /// w=...
    pub w: Option<u32>,

    /// Height (in pixels) of the source and destination rectangles.
    /// By default the full height is used.
    /// h=...
    pub h: Option<u32>,

    /// Left edge in pixels of the source rectangle
    /// X=...
    pub src_x: Option<u32>,
    /// Top edge in pixels of the source rectangle
    /// Y=...
    pub src_y: Option<u32>,

    /// Composition mode.
    /// Default is AlphaBlending
    /// C=...
    pub composition_mode: KittyFrameCompositionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageFrame {
    /// Left edge in pixels to update
    pub x: Option<u32>,
    /// Top edge in pixels to update
    pub y: Option<u32>,

    /// 1-based number of the frame which should be the base
    /// data for the new frame being created.
    /// If omitted, use background_pixel to specify color.
    /// c=...
    pub base_frame: Option<u32>,

    /// 1-based number of the frame which should be edited.
    /// If omitted, a new frame is created.
    /// r=...
    pub frame_number: Option<u32>,

    /// Gap in milliseconds of this frame from the next one.
    /// Zero or omitted values are interpreted as 40ms.
    /// z=...
    pub duration_ms: Option<u32>,

    /// Composition mode.
    /// Default is AlphaBlending
    /// X=...
    pub composition_mode: KittyFrameCompositionMode,

    /// Background color for pixels not specified in the frame data.
    /// If omitted, use a black, fully-transparent pixel (0)
    /// Y=...
    pub background_pixel: Option<u32>,
}

impl KittyImageFrame {
    fn from_keys(keys: &BTreeMap<&str, &str>) -> Option<Self> {
        Some(Self {
            x: geti(keys, "x"),
            y: geti(keys, "y"),
            base_frame: match geti(keys, "c") {
                None | Some(0) => None,
                n => n,
            },
            frame_number: match geti(keys, "r") {
                None | Some(0) => None,
                n => n,
            },
            duration_ms: match geti(keys, "Z") {
                None | Some(0) => None,
                n => n,
            },
            composition_mode: match geti(keys, "X") {
                None | Some(0) => KittyFrameCompositionMode::AlphaBlending,
                Some(1) => KittyFrameCompositionMode::Overwrite,
                _ => return None,
            },
            background_pixel: geti(keys, "Y"),
        })
    }

    fn to_keys(&self, keys: &mut BTreeMap<&'static str, String>) {
        set(keys, "x", &self.x);
        set(keys, "y", &self.y);
        set(keys, "c", &self.base_frame);
        set(keys, "r", &self.frame_number);
        set(keys, "Z", &self.duration_ms);
        match &self.composition_mode {
            KittyFrameCompositionMode::AlphaBlending => {}
            KittyFrameCompositionMode::Overwrite => {
                keys.insert("X", "1".to_string());
            }
        }
        set(keys, "Y", &self.background_pixel);
    }
}
