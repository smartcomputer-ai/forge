//! Durable publication receipts. Interrupted in-flight operations fail closed after
//! restart; they are never silently re-executed against a possibly edited target.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Serialize, Deserialize)]
pub struct Record {
    pub root: PathBuf,
    pub request: TransferRequest,
    pub status: TransferStatus,
    pub expires_at_ms: u64,
    pub stage_name: Option<String>,
    #[serde(default)]
    pub spool_directory: Option<PathBuf>,
}
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn write(directory: &Path, id: &str, record: &Record) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(io)?;
    serde_json::to_writer(&mut temporary, record).map_err(|e| invalid(&e.to_string()))?;
    temporary.as_file().sync_all().map_err(io)?;
    temporary
        .persist(directory.join(format!("{id}.json")))
        .map_err(|e| io(e.error))?;
    backend::sync_directory(directory).map_err(io)
}
pub fn read(directory: &Path, id: &str) -> Result<Option<Record>> {
    match File::open(directory.join(format!("{id}.json"))) {
        Ok(file) => Ok(Some(
            serde_json::from_reader(file.take(64 * 1024)).map_err(|e| invalid(&e.to_string()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}
