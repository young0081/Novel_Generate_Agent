use std::fs;
use std::io::Write;
use std::path::Path;

use na_common::{CoreError, ErrorKind, Result};

/// Compute the legacy length-tagged FNV-1a content key used by existing stores.
pub fn content_hash(bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:x}-{:x}", bytes.len(), hash)
}

/// Validate and return the byte length encoded in a content-object name.
pub fn validate_content_hash(hash: &str) -> Result<usize> {
    let invalid = || {
        CoreError::new(
            ErrorKind::Serialization,
            format!("invalid content-object hash {hash:?}"),
        )
    };
    let (length_hex, digest_hex) = hash.split_once('-').ok_or_else(invalid)?;
    if length_hex.is_empty()
        || digest_hex.is_empty()
        || length_hex.len() > 16
        || digest_hex.len() > 16
        || !length_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !digest_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || length_hex.bytes().any(|byte| byte.is_ascii_uppercase())
        || digest_hex.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(invalid());
    }
    let length = usize::from_str_radix(length_hex, 16).map_err(|_| invalid())?;
    let digest = u64::from_str_radix(digest_hex, 16).map_err(|_| invalid())?;
    if format!("{length:x}") != length_hex || format!("{digest:x}") != digest_hex {
        return Err(invalid());
    }
    Ok(length)
}

/// Read and verify one regular content-addressed object.
pub fn read_content_object(objects_dir: &Path, hash: &str) -> Result<Vec<u8>> {
    let expected_length = validate_content_hash(hash)?;
    let canonical_dir = validate_objects_dir(objects_dir)?;
    let path = objects_dir.join(hash);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        CoreError::from(error).with_context(format!("reading metadata for object {hash}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(CoreError::sandbox(format!(
            "content object {hash:?} is not a regular file"
        )));
    }
    let canonical_path = fs::canonicalize(&path)
        .map_err(|error| CoreError::from(error).with_context(format!("resolving object {hash}")))?;
    if canonical_path.parent() != Some(canonical_dir.as_path()) {
        return Err(CoreError::sandbox(format!(
            "content object {hash:?} escapes its object directory"
        )));
    }

    let bytes = fs::read(&path)
        .map_err(|error| CoreError::from(error).with_context(format!("reading object {hash}")))?;
    if bytes.len() != expected_length || content_hash(&bytes) != hash {
        return Err(CoreError::new(
            ErrorKind::Serialization,
            format!("content object {hash:?} failed integrity verification"),
        ));
    }
    Ok(bytes)
}

/// Create a content-addressed object atomically, or verify the existing object.
pub fn write_content_object(objects_dir: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    validate_content_hash(hash)?;
    if content_hash(bytes) != hash {
        return Err(CoreError::invalid_input(format!(
            "object name {hash:?} does not match its content"
        )));
    }
    validate_objects_dir(objects_dir)?;
    let path = objects_dir.join(hash);
    match fs::symlink_metadata(&path) {
        Ok(_) => return read_content_object(objects_dir, hash).map(|_| ()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(
                CoreError::from(error).with_context(format!("reading metadata for object {hash}"))
            )
        }
    }

    let mut temporary = tempfile::NamedTempFile::new_in(objects_dir).map_err(|error| {
        CoreError::from(error).with_context(format!("creating temporary object {hash}"))
    })?;
    temporary.write_all(bytes).map_err(|error| {
        CoreError::from(error).with_context(format!("writing temporary object {hash}"))
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        CoreError::from(error).with_context(format!("flushing temporary object {hash}"))
    })?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => sync_directory(objects_dir).map_err(|error| {
            error.with_context(format!("syncing object directory after creating {hash}"))
        }),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            drop(error.file);
            read_content_object(objects_dir, hash).map(|_| ())
        }
        Err(error) => {
            Err(CoreError::from(error.error)
                .with_context(format!("persisting content object {hash}")))
        }
    }
}

pub(crate) fn atomic_write_file(path: &Path, bytes: &[u8], description: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        CoreError::from(error).with_context(format!("creating directory for {description}"))
    })?;
    validate_real_directory(parent, description)?;
    validate_atomic_destination(path, description)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CoreError::from(error).with_context(format!("creating temporary {description}"))
    })?;
    temporary.write_all(bytes).map_err(|error| {
        CoreError::from(error).with_context(format!("writing temporary {description}"))
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        CoreError::from(error).with_context(format!("flushing temporary {description}"))
    })?;
    // Recheck immediately before the rename. In particular, never accept a
    // symlink that appeared while the temporary file was being written.
    validate_atomic_destination(path, description)?;
    temporary.persist(path).map_err(|error| {
        CoreError::from(error.error).with_context(format!("replacing {description}"))
    })?;
    sync_directory(parent)
        .map_err(|error| error.with_context(format!("syncing directory for {description}")))
}

fn validate_atomic_destination(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CoreError::sandbox(format!(
            "refusing to replace symlink used as {description}"
        ))),
        Ok(metadata) if !metadata.file_type().is_file() => Err(CoreError::conflict(format!(
            "refusing to replace non-file used as {description}"
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoreError::from(error)
            .with_context(format!("inspecting destination for {description}"))),
    }
}

fn validate_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CoreError::from(error).with_context(format!("inspecting directory for {description}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(CoreError::sandbox(format!(
            "directory for {description} is not a real directory"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::from)
}

// Opening directories for flushing requires platform-specific Windows flags.
// Atomic replacement is still provided there; Unix additionally persists the
// directory entry across a power loss.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn validate_objects_dir(objects_dir: &Path) -> Result<std::path::PathBuf> {
    let metadata = fs::symlink_metadata(objects_dir).map_err(|error| {
        CoreError::from(error).with_context("reading content-object directory metadata")
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(CoreError::sandbox(
            "content-object directory is not a regular directory",
        ));
    }
    fs::canonicalize(objects_dir)
        .map_err(|error| CoreError::from(error).with_context("resolving content-object directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("na_objects_{tag}_"))
            .tempdir()
            .unwrap()
    }

    #[test]
    fn writes_reads_and_rejects_invalid_names() {
        let root = temp_dir("roundtrip");
        let objects = root.path().join("objects");
        fs::create_dir(&objects).unwrap();
        let bytes = b"chapter content";
        let hash = content_hash(bytes);

        write_content_object(&objects, &hash, bytes).unwrap();
        write_content_object(&objects, &hash, bytes).unwrap();
        assert_eq!(read_content_object(&objects, &hash).unwrap(), bytes);
        assert!(read_content_object(&objects, "../outside").is_err());
        assert!(read_content_object(&objects, "0-ABC").is_err());
        assert!(read_content_object(&objects, "00-cbf29ce484222325").is_err());
    }

    #[test]
    fn corrupt_existing_object_is_never_reused() {
        let root = temp_dir("corrupt");
        let objects = root.path().join("objects");
        fs::create_dir(&objects).unwrap();
        let expected = b"expected";
        let hash = content_hash(expected);
        fs::write(objects.join(&hash), b"tampered").unwrap();

        assert!(read_content_object(&objects, &hash).is_err());
        assert!(write_content_object(&objects, &hash, expected).is_err());
        assert_eq!(fs::read(objects.join(hash)).unwrap(), b"tampered");
    }
}
