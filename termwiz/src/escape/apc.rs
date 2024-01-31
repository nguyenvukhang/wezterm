use crate::escape::osc::base64_decode;
use std::fmt::Formatter;
use std::io::{Read, Seek};

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
