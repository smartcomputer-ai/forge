//! Canonical read-file operation.

use serde::{Deserialize, Serialize};

use crate::{
    error::ToolResult,
    fs::{FsError, FsPath, FsToolContext},
};

use super::{invalid_request, resolve_path};

pub const DEFAULT_READ_FILE_LINE_LIMIT: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadFileArgs {
    pub path: FsPath,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub path: FsPath,
    pub resolved_path: FsPath,
    pub text: String,
    pub line_numbered_text: String,
    pub line_start: usize,
    pub line_count: usize,
    pub total_lines: usize,
    pub truncated: bool,
    pub bytes_read: usize,
}

pub async fn invoke_read_file(
    ctx: &FsToolContext,
    args: ReadFileArgs,
) -> ToolResult<ReadFileResult> {
    let resolved_path = resolve_path(ctx, &args.path)?;
    let offset = args.offset.unwrap_or(1);
    if offset == 0 {
        return Err(invalid_request("read_file offset must be 1 or greater"));
    }
    let limit = args.limit.unwrap_or(DEFAULT_READ_FILE_LINE_LIMIT);
    if limit == 0 {
        return Err(invalid_request("read_file limit must be 1 or greater"));
    }

    // Ranged read: an oversized file is truncated at the source (when the
    // backend supports ranges) and rejected with its true size instead of
    // being transferred in full.
    let read = ctx
        .fs
        .read_file_range(
            &resolved_path,
            0,
            Some(ctx.limits.max_file_read_bytes.saturating_add(1)),
        )
        .await?;
    if read.truncated || read.file_size > ctx.limits.max_file_read_bytes {
        return Err(invalid_request(format!(
            "read_file would read {} bytes, exceeding max_file_read_bytes={}",
            read.file_size, ctx.limits.max_file_read_bytes
        )));
    }
    let bytes_read = read.bytes.len();

    let contents = String::from_utf8(read.bytes).map_err(FsError::invalid_data)?;
    let lines = contents.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let start_index = offset - 1;
    let selected = lines
        .iter()
        .enumerate()
        .skip(start_index)
        .take(limit)
        .map(|(index, line)| (index + 1, *line))
        .collect::<Vec<_>>();

    let text = selected
        .iter()
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n");
    let line_numbered_text = selected
        .iter()
        .map(|(line_number, line)| format!("{line_number:>6} | {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let line_count = selected.len();
    let truncated = start_index < total_lines && start_index + line_count < total_lines;

    Ok(ReadFileResult {
        path: args.path,
        resolved_path,
        text,
        line_numbered_text,
        line_start: offset,
        line_count,
        total_lines,
        truncated,
        bytes_read,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use engine::storage::InMemoryBlobStore;

    use super::*;
    use crate::{
        error::ToolError,
        fs::{CreateDirectoryOptions, FileSystem, InMemoryFileSystem},
        limits::ToolLimits,
    };

    fn context(fs: Arc<dyn FileSystem>) -> FsToolContext {
        FsToolContext::new(fs, Arc::new(InMemoryBlobStore::new()))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_read_file_resolves_relative_paths_against_context_cwd() {
        let fs = InMemoryFileSystem::full_access();
        fs.create_directory(
            &FsPath::new("/workspace").expect("dir"),
            CreateDirectoryOptions::single(),
        )
        .await
        .expect("create directory");
        fs.write_file(
            &FsPath::new("/workspace/file.txt").expect("file path"),
            b"hello".to_vec(),
        )
        .await
        .expect("write file");
        let ctx = context(Arc::new(fs)).with_cwd(FsPath::new("/workspace").expect("cwd"));

        let result = invoke_read_file(
            &ctx,
            ReadFileArgs {
                path: FsPath::new("file.txt").expect("relative path"),
                offset: None,
                limit: None,
            },
        )
        .await
        .expect("read file");

        assert_eq!(
            result.resolved_path,
            FsPath::new("/workspace/file.txt").unwrap()
        );
        assert_eq!(result.text, "hello");
        assert_eq!(result.line_numbered_text, "     1 | hello");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_read_file_applies_offset_and_limit() {
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(
            &FsPath::new("/file.txt").expect("file path"),
            b"one\ntwo\nthree\nfour".to_vec(),
        )
        .await
        .expect("write file");
        let ctx = context(Arc::new(fs));

        let result = invoke_read_file(
            &ctx,
            ReadFileArgs {
                path: FsPath::new("/file.txt").expect("path"),
                offset: Some(2),
                limit: Some(2),
            },
        )
        .await
        .expect("read file");

        assert_eq!(result.text, "two\nthree");
        assert_eq!(result.line_numbered_text, "     2 | two\n     3 | three");
        assert_eq!(result.line_start, 2);
        assert_eq!(result.line_count, 2);
        assert_eq!(result.total_lines, 4);
        assert!(result.truncated);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_read_file_enforces_max_file_read_bytes() {
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(
            &FsPath::new("/file.txt").expect("file path"),
            b"hello".to_vec(),
        )
        .await
        .expect("write file");
        let ctx = context(Arc::new(fs)).with_limits(ToolLimits {
            max_file_read_bytes: 4,
            ..ToolLimits::default()
        });

        let error = invoke_read_file(
            &ctx,
            ReadFileArgs {
                path: FsPath::new("/file.txt").expect("path"),
                offset: None,
                limit: None,
            },
        )
        .await
        .expect_err("read should fail");

        assert!(matches!(error, ToolError::InvalidRequest { .. }));
    }

    /// A backend with native range support truncates at the source; the tool
    /// must reject the oversized file using the reported true size without a
    /// full transfer.
    struct RangedOnlyFileSystem {
        file_size: u64,
    }

    #[async_trait::async_trait]
    impl FileSystem for RangedOnlyFileSystem {
        fn access_policy(&self) -> crate::fs::FileAccessPolicy {
            InMemoryFileSystem::full_access().access_policy()
        }

        async fn read_file(&self, _path: &FsPath) -> crate::fs::FsResult<Vec<u8>> {
            panic!("ranged backend must not be asked for a full transfer");
        }

        async fn read_file_range(
            &self,
            _path: &FsPath,
            offset: u64,
            max_bytes: Option<u64>,
        ) -> crate::fs::FsResult<crate::fs::FsRangedRead> {
            let take = max_bytes.expect("read tool always bounds the range") as usize;
            let returned = take.min((self.file_size - offset) as usize);
            Ok(crate::fs::FsRangedRead {
                bytes: vec![b'a'; returned],
                file_size: self.file_size,
                truncated: (offset + returned as u64) < self.file_size,
            })
        }

        async fn write_file(&self, _path: &FsPath, _contents: Vec<u8>) -> crate::fs::FsResult<()> {
            unimplemented!()
        }

        async fn create_directory(
            &self,
            _path: &FsPath,
            _options: CreateDirectoryOptions,
        ) -> crate::fs::FsResult<()> {
            unimplemented!()
        }

        async fn get_metadata(
            &self,
            _path: &FsPath,
        ) -> crate::fs::FsResult<crate::fs::FileMetadata> {
            unimplemented!()
        }

        async fn read_directory(
            &self,
            _path: &FsPath,
        ) -> crate::fs::FsResult<Vec<crate::fs::ReadDirectoryEntry>> {
            unimplemented!()
        }

        async fn remove(
            &self,
            _path: &FsPath,
            _options: crate::fs::RemoveOptions,
        ) -> crate::fs::FsResult<()> {
            unimplemented!()
        }

        async fn copy(
            &self,
            _source_path: &FsPath,
            _destination_path: &FsPath,
            _options: crate::fs::CopyOptions,
        ) -> crate::fs::FsResult<()> {
            unimplemented!()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_read_file_rejects_oversized_files_without_full_transfer() {
        let ctx =
            context(Arc::new(RangedOnlyFileSystem { file_size: 100 })).with_limits(ToolLimits {
                max_file_read_bytes: 10,
                ..ToolLimits::default()
            });

        let error = invoke_read_file(
            &ctx,
            ReadFileArgs {
                path: FsPath::new("/huge.bin").expect("path"),
                offset: None,
                limit: None,
            },
        )
        .await
        .expect_err("oversized read should fail");

        let ToolError::InvalidRequest { message } = error else {
            panic!("expected invalid request");
        };
        assert!(message.contains("100 bytes"), "message: {message}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_read_file_accepts_a_file_exactly_at_the_cap() {
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(&FsPath::new("/file.txt").expect("path"), b"12345".to_vec())
            .await
            .expect("write file");
        let ctx = context(Arc::new(fs)).with_limits(ToolLimits {
            max_file_read_bytes: 5,
            ..ToolLimits::default()
        });

        let result = invoke_read_file(
            &ctx,
            ReadFileArgs {
                path: FsPath::new("/file.txt").expect("path"),
                offset: None,
                limit: None,
            },
        )
        .await
        .expect("read at cap");

        assert_eq!(result.text, "12345");
        assert_eq!(result.bytes_read, 5);
    }
}
