//! OS boundary for confined traversal and atomic publication. Wire paths and inventories
//! contain no descriptors, inode numbers, or Unix permission bits. A Windows backend can
//! implement this interface with directory handles, reparse-point checks and native rename.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "backend/unix.rs"]
mod unix;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::*;
