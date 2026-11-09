//! Shared memory for the `shm` kind: a memfd (Linux; an unlinked POSIX
//! shared memory object on other Unix) or a file mapping (Windows), mapped
//! by both processes.
use crate::Handle;
use std::io;

/// Memory mapped into this process, shared with the other.
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
}
// Safety: the mapping is plain memory; who writes when is the protocol's
// (a buffer is the client's until presented, the host's until released).
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    /// New shared memory of `len` bytes, zeroed: its handle (to send) and
    /// this process's mapping of it.
    pub fn create(len: usize) -> io::Result<(Handle, Mapping)> {
        let handle = imp::create(len)?;
        let mapping = Self::map(&handle, len)?;
        Ok((handle, mapping))
    }
    /// Map `len` bytes of the memory `handle` names.
    pub fn map(handle: &Handle, len: usize) -> io::Result<Mapping> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an empty mapping",
            ));
        }
        let ptr = imp::map(handle, len)?;
        Ok(Mapping { ptr, len })
    }
    /// Its length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }
    /// Whether it is empty (never: `map` refuses).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Its bytes.
    ///
    /// # Safety
    /// The other process may be writing them: read only what the protocol
    /// says is this side's (a presented buffer, on the host).
    pub unsafe fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
    /// Its bytes, to write.
    ///
    /// # Safety
    /// Write only what is this side's (an acquired buffer, on the client),
    /// and hold no other view of them meanwhile.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn bytes_mut(&self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        imp::unmap(self.ptr, self.len);
    }
}

#[cfg(unix)]
mod imp {
    use crate::Handle;
    use std::{
        io,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
    };

    pub fn create(len: usize) -> io::Result<Handle> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let fd = {
            let fd = unsafe { libc::memfd_create(c"zgx-surface".as_ptr(), libc::MFD_CLOEXEC) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            fd
        };
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let fd = {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let name = std::ffi::CString::new(format!(
                "/zgx-surface-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .unwrap();
            let fd = unsafe {
                libc::shm_open(
                    name.as_ptr(),
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            unsafe { libc::shm_unlink(name.as_ptr()) };
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            fd
        };
        // Safety: a new descriptor this owns.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(owned)
    }
    pub fn map(handle: &Handle, len: usize) -> io::Result<*mut u8> {
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                handle.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(ptr.cast())
    }
    pub fn unmap(ptr: *mut u8, len: usize) {
        unsafe { libc::munmap(ptr.cast(), len) };
    }
}

#[cfg(windows)]
mod imp {
    use crate::Handle;
    use std::{
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Memory::{
            CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            PAGE_READWRITE, UnmapViewOfFile,
        },
    };

    pub fn create(len: usize) -> io::Result<Handle> {
        let len = len as u64;
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                (len >> 32) as u32,
                len as u32,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // Safety: a new handle this owns.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle as _) })
    }
    pub fn map(handle: &Handle, len: usize) -> io::Result<*mut u8> {
        let view =
            unsafe { MapViewOfFile(handle.as_raw_handle() as _, FILE_MAP_ALL_ACCESS, 0, 0, len) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(view.Value.cast())
    }
    pub fn unmap(ptr: *mut u8, _len: usize) {
        unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: ptr.cast() }) };
    }
}
