use std::{
    ffi::{CStr, CString},
    fs::{File, Metadata},
    io,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path},
};

pub struct Directory(File);
#[derive(Clone, Debug)]
pub struct Observation(Metadata);
impl Observation {
    pub fn is_dir(&self) -> bool {
        self.0.is_dir()
    }
    pub fn size(&self) -> u64 {
        self.0.len()
    }
    pub fn executable(&self) -> bool {
        self.0.mode() & 0o111 != 0
    }
    pub fn matches(&self, other: &Self) -> bool {
        let (a, b) = (&self.0, &other.0);
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.mode() == b.mode()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
}
fn c(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}
fn owned(fd: i32) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: successful calls return one newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn check(meta: Metadata) -> io::Result<Observation> {
    if meta.is_file() || meta.is_dir() {
        Ok(Observation(meta))
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "scoped filesystem access does not support symlinks or special files",
        ))
    }
}
pub fn observe(file: &File) -> io::Result<Observation> {
    check(file.metadata()?)
}
pub fn set_executable(file: &File, executable: bool) -> io::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(if executable {
        0o755
    } else {
        0o644
    }))
}
/// Streaming directory enumeration; cleanup never collects an entire retired tree.
struct Entries(*mut libc::DIR);
impl Drop for Entries {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
impl Iterator for Entries {
    type Item = io::Result<CString>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            let entry = unsafe { libc::readdir(self.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return (error.raw_os_error() != Some(0)).then_some(Err(error));
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            return Some(Ok(name.to_owned()));
        }
    }
}
fn entries(file: &File) -> io::Result<Entries> {
    let dot = c(".")?;
    let fd = owned(unsafe {
        libc::openat(
            file.as_raw_fd(),
            dot.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    })?
    .into_raw_fd();
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(error);
    }
    Ok(Entries(stream))
}

pub fn is_path_violation(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
}
pub fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}
pub fn sync_directory(path: &Path) -> io::Result<()> {
    Directory::anchor(path)?.sync()
}

impl Directory {
    pub fn anchor(root: &Path) -> io::Result<Self> {
        // Configured roots may contain platform aliases (/var on macOS). Resolve this
        // administrator-owned anchor once; user selections are always opened relative to it.
        let root = root.canonicalize()?;
        let mut dir = Self(
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")?,
        );
        for part in root.components() {
            if let Component::Normal(name) = part {
                let name = name
                    .to_str()
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
                dir = dir.child(name)?;
            }
        }
        Ok(dir)
    }
    pub fn sync(&self) -> io::Result<()> {
        self.0.sync_all()
    }
    pub fn clone_dir(&self) -> io::Result<Self> {
        Ok(Self(self.0.try_clone()?))
    }
    pub fn child(&self, name: &str) -> io::Result<Self> {
        self.child_name(&c(name)?)
    }
    fn child_name(&self, name: &CStr) -> io::Result<Self> {
        // SAFETY: valid descriptor and NUL-terminated name. Never follow a symlink.
        let file = owned(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        Ok(Self(file))
    }
    pub fn parent(&self, path: &str) -> io::Result<(Self, String)> {
        self.parent_with_creation(path, false)
    }
    /// Prepare a destination without following symlinks in existing or raced-in parents.
    pub fn ensure_parent(&self, path: &str) -> io::Result<(Self, String)> {
        self.parent_with_creation(path, true)
    }
    fn parent_with_creation(&self, path: &str, create_missing: bool) -> io::Result<(Self, String)> {
        let mut parts = path.split('/').collect::<Vec<_>>();
        let name = parts.pop().unwrap_or("").to_owned();
        let mut dir = self.clone_dir()?;
        for part in parts {
            dir = match dir.child(part) {
                Ok(child) => child,
                Err(error) if create_missing && error.kind() == io::ErrorKind::NotFound => {
                    let child = match dir.mkdir(part, false) {
                        Ok(child) => child,
                        // Another creator may win the mkdir race. Reopen with the same
                        // directory-only, no-follow checks used for existing parents.
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            dir.child(part)?
                        }
                        Err(error) => return Err(error),
                    };
                    dir.sync()?;
                    child
                }
                Err(error) => return Err(error),
            };
        }
        Ok((dir, if name.is_empty() { ".".into() } else { name }))
    }
    pub fn open(&self, path: &str) -> io::Result<File> {
        self.open_kind(path, false)
    }
    pub fn metadata(&self, path: &str) -> io::Result<File> {
        self.open_kind(path, true)
    }
    fn open_kind(&self, path: &str, metadata_only: bool) -> io::Result<File> {
        let (dir, name) = self.parent(path)?;
        let name = c(&name)?;
        #[cfg(target_os = "linux")]
        let access = if metadata_only {
            libc::O_PATH
        } else {
            libc::O_RDONLY
        };
        #[cfg(target_os = "macos")]
        let access = if metadata_only {
            libc::O_EVTONLY
        } else {
            libc::O_RDONLY
        };
        let file = owned(unsafe {
            libc::openat(
                dir.0.as_raw_fd(),
                name.as_ptr(),
                access | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        })?;
        observe(&file)?;
        Ok(file)
    }
    /// Inspect replacement targets without requiring read permission on their content.
    pub fn target_exists(&self, name: &str) -> io::Result<bool> {
        let name = c(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(error)
            };
        }
        let stat = unsafe { stat.assume_init() };
        let kind = stat.st_mode & libc::S_IFMT;
        if kind != libc::S_IFDIR && kind != libc::S_IFREG {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "target is a symlink or special file",
            ));
        }
        Ok(true)
    }
    pub fn names(file: &File, limit: usize) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for name in entries(file)? {
            if names.len() >= limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "directory entry limit exceeded",
                ));
            }
            names.push(
                name?.into_string().map_err(|_| {
                    io::Error::new(io::ErrorKind::Unsupported, "non-UTF-8 filename")
                })?,
            );
        }
        names.sort();
        Ok(names)
    }
    pub fn mkdir(&self, name: &str, private: bool) -> io::Result<Self> {
        let cname = c(name)?;
        if unsafe {
            libc::mkdirat(
                self.0.as_raw_fd(),
                cname.as_ptr(),
                if private { 0o700 } else { 0o755 },
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        self.child(name)
    }
    pub fn create(&self, path: &str) -> io::Result<File> {
        let (dir, name) = self.parent(path)?;
        let name = c(&name)?;
        owned(unsafe {
            libc::openat(
                dir.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })
    }
    pub fn publish(&self, stage: &Self, target: &str, replace: bool) -> io::Result<bool> {
        let old = c("tree")?;
        let new = c(target)?;
        let rename = |exchange: bool| {
            #[cfg(target_os = "linux")]
            let result = unsafe {
                libc::renameat2(
                    stage.0.as_raw_fd(),
                    old.as_ptr(),
                    self.0.as_raw_fd(),
                    new.as_ptr(),
                    if exchange {
                        libc::RENAME_EXCHANGE
                    } else {
                        libc::RENAME_NOREPLACE
                    },
                )
            };
            #[cfg(target_os = "macos")]
            let result = unsafe {
                libc::renameatx_np(
                    stage.0.as_raw_fd(),
                    old.as_ptr(),
                    self.0.as_raw_fd(),
                    new.as_ptr(),
                    if exchange {
                        libc::RENAME_SWAP
                    } else {
                        libc::RENAME_EXCL
                    },
                )
            };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        match rename(false) {
            Ok(()) => Ok(false),
            Err(error) if replace && error.kind() == io::ErrorKind::AlreadyExists => {
                rename(true)?;
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }
    pub fn remove_tree(&self, name: &str) -> io::Result<()> {
        struct Frame {
            name: CString,
            directory: Directory,
            entries: Entries,
        }
        let name = c(name)?;
        let unlink = |parent: &Directory, name: &CStr, flags: i32| -> io::Result<()> {
            if unsafe { libc::unlinkat(parent.0.as_raw_fd(), name.as_ptr(), flags) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        let directory = match self.child_name(&name) {
            Ok(dir) => dir,
            Err(_) => return unlink(self, &name, 0),
        };
        let mut stack = vec![Frame {
            name,
            entries: entries(&directory.0)?,
            directory,
        }];
        while let Some(frame) = stack.last_mut() {
            if let Some(name) = frame.entries.next() {
                let name = name?;
                match frame.directory.child_name(&name) {
                    Ok(directory) => {
                        if stack.len() >= 256 {
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                "retirement cleanup depth limit exceeded",
                            ));
                        }
                        stack.push(Frame {
                            name,
                            entries: entries(&directory.0)?,
                            directory,
                        });
                    }
                    Err(_) => unlink(&frame.directory, &name, 0)?,
                }
            } else {
                let frame = stack.pop().unwrap();
                let parent = stack.last().map_or(self, |frame| &frame.directory);
                unlink(parent, &frame.name, libc::AT_REMOVEDIR)?;
            }
        }
        Ok(())
    }
}
