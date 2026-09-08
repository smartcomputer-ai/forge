//! Linux descriptor-relative transfer. Ordinary lexical filesystem resolution is insufficient here.
use super::*;

#[cfg(not(target_os = "linux"))]
pub(super) fn capture(
    _: &Path,
    _: &Path,
    _: CaptureParams,
) -> Result<CaptureResponse, EnvironmentProtocolError> {
    Err(EnvironmentProtocolError::new(
        EnvironmentProtocolErrorCode::Unsupported,
        "transfer requires Linux openat2 and renameat2",
    ))
}
#[cfg(not(target_os = "linux"))]
pub(super) fn materialize(
    _: &Path,
    _: &Path,
    _: MaterializeParams,
) -> Result<MaterializeResponse, EnvironmentProtocolError> {
    Err(EnvironmentProtocolError::new(
        EnvironmentProtocolErrorCode::Unsupported,
        "transfer requires Linux openat2 and renameat2",
    ))
}
#[cfg(target_os = "linux")]
pub(super) use linux::{capture, materialize};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        collections::BTreeMap,
        ffi::CString,
        fs::{File, Metadata},
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{MetadataExt, PermissionsExt},
            },
        },
    };
    type Result<T> = std::result::Result<T, EnvironmentProtocolError>;
    fn fail(code: EnvironmentProtocolErrorCode, message: &str) -> EnvironmentProtocolError {
        EnvironmentProtocolError::new(code, message)
    }
    fn invalid(message: &str) -> EnvironmentProtocolError {
        fail(EnvironmentProtocolErrorCode::InvalidRequest, message)
    }
    fn io(e: io::Error) -> EnvironmentProtocolError {
        let code = match e.raw_os_error() {
            Some(libc::ELOOP | libc::EXDEV | libc::ENOTDIR) => {
                EnvironmentProtocolErrorCode::Forbidden
            }
            Some(libc::ENOSYS | libc::EOPNOTSUPP) => EnvironmentProtocolErrorCode::Unsupported,
            Some(libc::EEXIST) => EnvironmentProtocolErrorCode::Conflict,
            Some(libc::ENOENT) => EnvironmentProtocolErrorCode::NotFound,
            _ => EnvironmentProtocolErrorCode::Internal,
        };
        EnvironmentProtocolError::new(code, e.to_string())
    }
    fn c(path: &Path) -> Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("NUL path"))
    }
    // Linux UAPI open_how. NO_XDEV excludes mount crossings as well as symlink escapes.
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    fn open(dir: &File, path: &Path, flags: i32, mode: u64) -> Result<File> {
        let path = c(path)?;
        let how = OpenHow {
            flags: (flags
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if flags & libc::O_PATH == 0 {
                    libc::O_NONBLOCK
                } else {
                    0
                }) as u64,
            mode,
            resolve: 0x01 | 0x04 | 0x08,
        };
        // SAFETY: valid C string and initialized UAPI structure; returned fd is owned once.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                dir.as_raw_fd(),
                path.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if fd < 0 {
            return Err(io(io::Error::last_os_error()));
        }
        Ok(unsafe { File::from_raw_fd(fd as i32) })
    }
    fn object(dir: &File, path: &Path) -> Result<File> {
        // Inspect a pinned object without opening devices/FIFOs for I/O. Reopening
        // through procfs retains that inode even if the directory entry is replaced.
        let pinned = open(dir, path, libc::O_PATH, 0)?;
        let metadata = pinned.metadata().map_err(io)?;
        if metadata.file_type().is_symlink() {
            return Err(fail(
                EnvironmentProtocolErrorCode::Forbidden,
                "symlinks cannot transfer",
            ));
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(fail(
                EnvironmentProtocolErrorCode::Unsupported,
                "only regular files and directories can transfer",
            ));
        }
        File::open(format!("/proc/self/fd/{}", pinned.as_raw_fd())).map_err(io)
    }

    fn anchor(root: &Path) -> Result<File> {
        // Open every configured root component without following symlinks. Mount crossings
        // are allowed only while establishing the administrator-configured root.
        use std::os::unix::fs::OpenOptionsExt;
        let mut dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")
            .map_err(io)?;
        if !root.is_absolute() {
            return Err(invalid("filesystem root must be absolute"));
        }
        for part in root.components() {
            if let Component::Normal(name) = part {
                let name = c(Path::new(name))?;
                let fd = unsafe {
                    libc::openat(
                        dir.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(io(io::Error::last_os_error()));
                }
                dir = unsafe { File::from_raw_fd(fd) };
            }
        }
        Ok(dir)
    }
    struct Budget {
        limits: TransferLimits,
        start: Instant,
        entries: u32,
        bytes: u64,
    }
    impl Budget {
        fn new(limits: TransferLimits) -> Result<Self> {
            if limits.max_entries == 0
                || limits.max_entries > MAX_TRANSFER_ENTRIES
                || limits.max_depth > MAX_TRANSFER_DEPTH
                || limits.max_file_bytes > MAX_TRANSFER_BYTES
                || limits.max_total_bytes > MAX_TRANSFER_BYTES
                || limits.max_duration_ms == 0
                || limits.max_duration_ms > MAX_TRANSFER_DURATION_MS
            {
                return Err(invalid(
                    "transfer limits exceed protocol ceilings or are invalid",
                ));
            }
            Ok(Self {
                limits,
                start: Instant::now(),
                entries: 0,
                bytes: 0,
            })
        }
        fn time(&self) -> Result<()> {
            if self.start.elapsed() >= Duration::from_millis(self.limits.max_duration_ms) {
                return Err(fail(
                    EnvironmentProtocolErrorCode::Timeout,
                    "transfer deadline exceeded",
                ));
            }
            Ok(())
        }
        fn entry(&mut self, path: &str, bytes: u64) -> Result<()> {
            self.time()?;
            let depth = if path.is_empty() {
                0
            } else {
                path.split('/').count() as u32
            };
            self.entries += 1;
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .ok_or_else(|| invalid("byte overflow"))?;
            if self.entries > self.limits.max_entries
                || depth > self.limits.max_depth
                || bytes > self.limits.max_file_bytes
                || self.bytes > self.limits.max_total_bytes
                || path.len() > MAX_TRANSFER_PATH_BYTES
            {
                return Err(invalid("transfer limit exceeded"));
            }
            Ok(())
        }
    }
    fn valid(path: &str) -> bool {
        path.is_empty()
            || (!path.contains(['\\', '\0'])
                && path
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != ".."))
    }
    fn same(a: &Metadata, b: &Metadata) -> bool {
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.mode() == b.mode()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    fn changed() -> EnvironmentProtocolError {
        fail(
            EnvironmentProtocolErrorCode::Conflict,
            "source changed during capture",
        )
    }
    fn visit(
        dir: &File,
        name: &Path,
        relative: String,
        budget: &mut Budget,
        entries: &mut Vec<TransferEntry>,
        observed: &mut Vec<(String, Metadata)>,
    ) -> Result<()> {
        budget.time()?;
        let mut file = object(dir, name)?;
        let before = file.metadata().map_err(io)?;
        if !before.is_dir() && !before.is_file() {
            return Err(fail(
                EnvironmentProtocolErrorCode::Unsupported,
                "only regular files and directories can transfer",
            ));
        }
        budget.entry(&relative, if before.is_file() { before.len() } else { 0 })?;
        let content = if before.is_file() {
            let mut bytes = Vec::with_capacity(before.len() as usize);
            let mut chunk = [0u8; 65536];
            loop {
                budget.time()?;
                // Read one extra byte to detect growth, without unbounded allocation.
                let remaining =
                    (before.len() + 1 - bytes.len() as u64).min(chunk.len() as u64) as usize;
                let n = file.read(&mut chunk[..remaining]).map_err(io)?;
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() as u64 > before.len() {
                    return Err(changed());
                }
            }
            if bytes.len() as u64 != before.len() {
                return Err(changed());
            }
            TransferContent::File {
                data: ByteChunk::from(bytes),
                executable: before.mode() & 0o111 != 0,
            }
        } else {
            TransferContent::Directory
        };
        entries.push(TransferEntry {
            path: relative.clone(),
            content,
        });
        if before.is_dir() {
            // procfs names this already-open directory, never an untrusted pathname.
            for entry in
                std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(io)?
            {
                budget.time()?;
                let entry = entry.map_err(io)?;
                let name = entry.file_name().into_string().map_err(|_| {
                    fail(
                        EnvironmentProtocolErrorCode::Unsupported,
                        "non-UTF8 filename",
                    )
                })?;
                if !valid(&name) {
                    return Err(invalid("unsupported filename"));
                }
                let child = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}/{name}")
                };
                visit(&file, Path::new(&name), child, budget, entries, observed)?;
            }
        }
        if !same(&before, &file.metadata().map_err(io)?) {
            return Err(changed());
        }
        observed.push((relative, before));
        Ok(())
    }
    #[cfg(test)]
    mod checks {
        use super::*;
        #[test]
        fn expired_transfer_and_changed_source_are_failures() {
            let mut budget = Budget::new(TransferLimits {
                max_entries: 1,
                max_depth: 0,
                max_file_bytes: 1,
                max_total_bytes: 1,
                max_duration_ms: 1,
            })
            .unwrap();
            budget.start = Instant::now() - Duration::from_secs(1);
            assert_eq!(
                budget.time().unwrap_err().code,
                EnvironmentProtocolErrorCode::Timeout
            );
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("file");
            std::fs::write(&path, b"a").unwrap();
            let before = std::fs::metadata(&path).unwrap();
            std::fs::write(&path, b"changed").unwrap();
            assert!(!same(&before, &std::fs::metadata(&path).unwrap()));
        }
    }

    pub fn capture(root: &Path, path: &Path, params: CaptureParams) -> Result<CaptureResponse> {
        let mut budget = Budget::new(params.limits)?;
        let root_fd = anchor(root)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| invalid("source outside root"))?;
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        let mut entries = Vec::new();
        let mut observed = Vec::new();
        visit(
            &root_fd,
            relative,
            String::new(),
            &mut budget,
            &mut entries,
            &mut observed,
        )?;
        // Reopen from the root to detect replacement/unlink after an entry was read.
        for (name, before) in observed {
            budget.time()?;
            let p = if name.is_empty() {
                relative.to_path_buf()
            } else {
                relative.join(name)
            };
            let file = object(&root_fd, &p)?;
            if !same(&before, &file.metadata().map_err(io)?) {
                return Err(changed());
            }
        }
        budget.time()?;
        Ok(CaptureResponse {
            source: EnvironmentPath::new(path.to_string_lossy())
                .map_err(|_| invalid("source path"))?,
            entries,
            bytes: budget.bytes,
        })
    }
    pub fn materialize(
        root: &Path,
        path: &Path,
        params: MaterializeParams,
    ) -> Result<MaterializeResponse> {
        let mut budget = Budget::new(params.limits)?;
        let mut kinds = BTreeMap::new();
        for entry in &params.entries {
            if !valid(&entry.path) || kinds.contains_key(&entry.path) {
                return Err(invalid("unsafe or duplicate entry path"));
            }
            if entry.path.is_empty() {
                if !kinds.is_empty() {
                    return Err(invalid("root must be first"));
                }
            } else {
                let parent = entry.path.rsplit_once('/').map_or("", |(p, _)| p);
                if kinds.get(parent) != Some(&true) {
                    return Err(invalid("missing directory parent"));
                }
            }
            let bytes = match &entry.content {
                TransferContent::File { data, .. } => data.as_slice().len() as u64,
                _ => 0,
            };
            budget.entry(&entry.path, bytes)?;
            kinds.insert(
                entry.path.clone(),
                matches!(entry.content, TransferContent::Directory),
            );
        }
        if !kinds.contains_key("") {
            return Err(invalid("missing selected root"));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| invalid("destination outside root"))?;
        if relative.as_os_str().is_empty() {
            return Err(invalid("cannot replace configured filesystem root"));
        }
        let root_fd = anchor(root)?;
        let parent_path = relative
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = open(&root_fd, parent_path, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        let name = relative
            .file_name()
            .ok_or_else(|| invalid("missing destination name"))?;
        // Reject unsupported selected targets, including symlinks. Publication itself never follows them.
        match object(&parent, Path::new(name)) {
            Ok(file) => {
                if params.on_existing == TransferOnExisting::Error {
                    return Err(fail(
                        EnvironmentProtocolErrorCode::Conflict,
                        "destination exists",
                    ));
                }
                let m = file.metadata().map_err(io)?;
                if !m.is_dir() && !m.is_file() {
                    return Err(fail(
                        EnvironmentProtocolErrorCode::Unsupported,
                        "unsupported destination",
                    ));
                }
            }
            Err(e) if e.code == EnvironmentProtocolErrorCode::NotFound => (),
            Err(e) => return Err(e),
        }
        let staging = tempfile::Builder::new()
            .prefix(".env-transfer-")
            .tempdir_in(format!("/proc/self/fd/{}", parent.as_raw_fd()))
            .map_err(io)?;
        let stage = open(
            &parent,
            Path::new(staging.path().file_name().unwrap()),
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        for entry in &params.entries {
            budget.time()?;
            let p = if entry.path.is_empty() {
                PathBuf::from("tree")
            } else {
                Path::new("tree").join(&entry.path)
            };
            match &entry.content {
                TransferContent::Directory => {
                    let ancestor = open(
                        &stage,
                        p.parent()
                            .filter(|p| !p.as_os_str().is_empty())
                            .unwrap_or(Path::new(".")),
                        libc::O_RDONLY | libc::O_DIRECTORY,
                        0,
                    )?;
                    let name = c(Path::new(p.file_name().unwrap()))?;
                    if unsafe { libc::mkdirat(ancestor.as_raw_fd(), name.as_ptr(), 0o755) } < 0 {
                        return Err(io(io::Error::last_os_error()));
                    }
                }
                TransferContent::File { data, executable } => {
                    let mut file = open(
                        &stage,
                        &p,
                        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                        0o600,
                    )?;
                    for chunk in data.as_slice().chunks(65536) {
                        budget.time()?;
                        file.write_all(chunk).map_err(io)?;
                    }
                    file.set_permissions(std::fs::Permissions::from_mode(if *executable {
                        0o755
                    } else {
                        0o644
                    }))
                    .map_err(io)?;
                }
            }
        }
        budget.time()?;
        let source = c(Path::new("tree"))?;
        let destination = c(Path::new(name))?;
        let rename = |flags: libc::c_uint| {
            let result = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    stage.as_raw_fd(),
                    source.as_ptr(),
                    parent.as_raw_fd(),
                    destination.as_ptr(),
                    flags,
                )
            };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        // NOREPLACE closes the absent-target race. EXCHANGE handles nonempty directories,
        // with no remove-then-rename gap and no fallback on unsupported filesystems.
        let retired_directory = match rename(libc::RENAME_NOREPLACE) {
            Ok(()) => None,
            Err(e)
                if e.raw_os_error() == Some(libc::EEXIST)
                    && params.on_existing == TransferOnExisting::Replace =>
            {
                rename(libc::RENAME_EXCHANGE).map_err(io)?;
                let actual = path
                    .parent()
                    .unwrap()
                    .join(staging.path().file_name().unwrap());
                let _ = staging.keep();
                Some(
                    EnvironmentPath::new(actual.to_string_lossy())
                        .map_err(|_| invalid("retirement path"))?,
                )
            }
            Err(e) => return Err(io(e)),
        };
        Ok(MaterializeResponse {
            destination: EnvironmentPath::new(path.to_string_lossy())
                .map_err(|_| invalid("destination path"))?,
            entries: budget.entries,
            bytes: budget.bytes,
            retired_directory,
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn limits() -> TransferLimits {
        TransferLimits {
            max_entries: 20,
            max_depth: 8,
            max_file_bytes: 1024,
            max_total_bytes: 4096,
            max_duration_ms: 5000,
        }
    }
    fn entry(path: &str, content: TransferContent) -> TransferEntry {
        TransferEntry {
            path: path.into(),
            content,
        }
    }
    fn file(path: &str) -> TransferEntry {
        entry(
            path,
            TransferContent::File {
                data: ByteChunk::from(vec![0, 255, 128, 10]),
                executable: true,
            },
        )
    }
    fn tree() -> Vec<TransferEntry> {
        vec![
            entry("", TransferContent::Directory),
            entry("empty", TransferContent::Directory),
            entry("nested", TransferContent::Directory),
            file("nested/bin"),
        ]
    }
    fn params(path: &str, entries: Vec<TransferEntry>) -> MaterializeParams {
        MaterializeParams {
            destination: EnvironmentPath::new(path).unwrap(),
            entries,
            limits: limits(),
            on_existing: TransferOnExisting::Replace,
        }
    }
    fn capture_params(path: &str) -> CaptureParams {
        CaptureParams {
            source: EnvironmentPath::new(path).unwrap(),
            limits: limits(),
        }
    }
    fn filesystem(root: &Path, writable: bool) -> LocalFileSystem {
        LocalFileSystem::new(root.into(), root.into(), writable)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transfer_roundtrips_file_tree_and_replaces_selected_target() {
        let temp = tempfile::tempdir().unwrap();
        let fs = filesystem(temp.path(), true);
        for entries in [vec![file("")], tree()] {
            let result = fs
                .materialize(params("selected", entries.clone()))
                .await
                .unwrap();
            assert_eq!(result.bytes, 4);
            let mut captured = fs.capture(capture_params("selected")).await.unwrap();
            captured.entries.sort_by(|a, b| a.path.cmp(&b.path));
            assert_eq!(captured.entries, entries);
            assert_eq!(captured.bytes, 4);
            if let Some(retired) = result.retired_directory {
                std::fs::remove_dir_all(retired.as_str()).unwrap();
            }
        }
        std::fs::write(temp.path().join("sibling"), b"keep").unwrap();
        std::fs::write(temp.path().join("selected/stale"), b"remove").unwrap();
        let mut collision = params("selected", vec![file("")]);
        collision.on_existing = TransferOnExisting::Error;
        assert_eq!(
            fs.materialize(collision).await.unwrap_err().code,
            EnvironmentProtocolErrorCode::Conflict
        );
        assert!(temp.path().join("selected/stale").exists());
        let response = fs.materialize(params("selected", tree())).await.unwrap();
        assert!(!temp.path().join("selected/stale").exists());
        assert_eq!(std::fs::read(temp.path().join("sibling")).unwrap(), b"keep");
        let retired = PathBuf::from(response.retired_directory.unwrap().as_str());
        assert!(retired.join("tree/stale").exists());
        assert_eq!(
            std::fs::metadata(temp.path().join("selected/nested/bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
        fs.materialize(params("selected", vec![file("")]))
            .await
            .unwrap();
        assert!(temp.path().join("selected").is_file());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transfer_rejects_unsafe_paths_links_special_files_and_read_only_writes() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"secret").unwrap();
        symlink(outside.path(), temp.path().join("link")).unwrap();
        let fs = filesystem(temp.path(), true);
        for path in ["link", "link/secret"] {
            assert_eq!(
                fs.capture(capture_params(path)).await.unwrap_err().code,
                EnvironmentProtocolErrorCode::Forbidden
            );
            assert_eq!(
                fs.materialize(params(path, vec![file("")]))
                    .await
                    .unwrap_err()
                    .code,
                EnvironmentProtocolErrorCode::Forbidden
            );
        }
        for path in ["../escape", "/escape", "a/../escape", "a//b", "a\\b", "./a"] {
            let mut entries = tree();
            entries.push(file(path));
            assert_eq!(
                fs.materialize(params("selected", entries))
                    .await
                    .unwrap_err()
                    .code,
                EnvironmentProtocolErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            fs.materialize(params(".", tree())).await.unwrap_err().code,
            EnvironmentProtocolErrorCode::InvalidRequest
        );
        assert_eq!(
            fs.capture(capture_params("../secret"))
                .await
                .unwrap_err()
                .code,
            EnvironmentProtocolErrorCode::Forbidden
        );
        let fifo = std::ffi::CString::new(temp.path().join("fifo").as_os_str().as_encoded_bytes())
            .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(
            fs.capture(capture_params("fifo")).await.unwrap_err().code,
            EnvironmentProtocolErrorCode::Unsupported
        );
        let read_only = filesystem(temp.path(), false);
        assert_eq!(
            read_only
                .materialize(params("selected", tree()))
                .await
                .unwrap_err()
                .code,
            EnvironmentProtocolErrorCode::CapabilityUnavailable
        );
        std::fs::write(temp.path().join("plain"), b"read").unwrap();
        let capture = read_only.capture(capture_params("plain")).await.unwrap();
        assert_eq!(
            capture.entries[0].content,
            TransferContent::File {
                data: ByteChunk::from(b"read".as_slice()),
                executable: false
            }
        );
        assert_eq!(
            std::fs::read(outside.path().join("secret")).unwrap(),
            b"secret"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transfer_failure_preserves_old_target_and_capture_never_returns_partial_tree() {
        let temp = tempfile::tempdir().unwrap();
        let fs = filesystem(temp.path(), true);
        fs.materialize(params("selected", tree())).await.unwrap();
        for bound in 0..4 {
            let mut request = params("selected", tree());
            match bound {
                0 => request.limits.max_entries = 1,
                1 => request.limits.max_depth = 0,
                2 => request.limits.max_file_bytes = 1,
                _ => request.limits.max_total_bytes = 1,
            }
            assert_eq!(
                fs.materialize(request).await.unwrap_err().code,
                EnvironmentProtocolErrorCode::InvalidRequest
            );
            let mut request = capture_params("selected");
            match bound {
                0 => request.limits.max_entries = 1,
                1 => request.limits.max_depth = 0,
                2 => request.limits.max_file_bytes = 1,
                _ => request.limits.max_total_bytes = 1,
            }
            assert_eq!(
                fs.capture(request).await.unwrap_err().code,
                EnvironmentProtocolErrorCode::InvalidRequest
            );
        }
        // Validation passes, then the filesystem rejects an overlong component during staging.
        let mut entries = tree();
        entries.push(file(&"x".repeat(256)));
        assert!(fs.materialize(params("selected", entries)).await.is_err());
        assert_eq!(
            fs.capture(capture_params("selected"))
                .await
                .unwrap()
                .entries
                .len(),
            4
        );
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "failed staging cleaned up"
        );
        symlink("missing", temp.path().join("selected/broken")).unwrap();
        assert_eq!(
            fs.capture(capture_params("selected"))
                .await
                .unwrap_err()
                .code,
            EnvironmentProtocolErrorCode::Forbidden
        );
        assert_eq!(
            fs.materialize(params("missing/child", tree()))
                .await
                .unwrap_err()
                .code,
            EnvironmentProtocolErrorCode::NotFound
        );
    }
}
