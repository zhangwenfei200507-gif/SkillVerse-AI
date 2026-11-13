use super::types::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const MAX_FILES: usize = 500;
pub const MAX_FILE: u64 = super::types::MAX_FILE_BYTES;
pub const MAX_TOTAL: u64 = 32 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Tree {
    pub kind: String,
    pub files: BTreeMap<String, String>,
}
pub fn relative(path: &str) -> Result<String> {
    let value = crate::github::types::safe_path(path)?;
    for part in value.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        if [
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        ]
        .contains(&stem.as_str())
        {
            return Err(fail(
                "UNSAFE_PATH",
                "A package path uses a Windows reserved device name.",
            ));
        }
    }
    Ok(value)
}
pub fn identifier(name: &str) -> Result<String> {
    let name = relative(name)?;
    if name.len() > 100
        || name.contains('/')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(fail("NAME", "Use a single safe directory identifier."));
    }
    Ok(name)
}
pub fn guard(profile: &Profile, path: &Path) -> Result<()> {
    let root = profile.approved_root(path).ok_or_else(|| {
        fail(
            "TARGET",
            "The target is outside this profile's approved roots.",
        )
    })?;
    if !root.is_absolute() || !crate::github::planner::contained_target(&root, path) {
        return Err(fail(
            "UNSAFE_PATH",
            "A symlink, junction, reparse point, or relative target was rejected.",
        ));
    }
    // Check the root itself as well as ancestors, including links above it.
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            if is_link(&meta) {
                return Err(fail(
                    "UNSAFE_PATH",
                    "A target ancestor is a link or reparse point.",
                ));
            }
        }
    }
    Ok(())
}
fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn read_regular(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| fail("FILESYSTEM", "A target could not be inspected."))?;
    if is_link(&meta) || !meta.is_file() || meta.len() > MAX_FILE {
        return Err(fail(
            "UNSAFE_FILE",
            "Only bounded ordinary files are supported.",
        ));
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = opts.open(path).map_err(|_| {
        fail(
            "PERMISSION",
            "A file could not be opened without following links.",
        )
    })?;
    if is_link(
        &file
            .metadata()
            .map_err(|_| fail("FILESYSTEM", "File metadata is unavailable."))?,
    ) {
        return Err(fail("UNSAFE_FILE", "A linked file was rejected."));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("FILESYSTEM", "A file could not be read."))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(fail("LIMIT", "File size exceeds the limit."));
    }
    Ok(bytes)
}
pub fn read(profile: &Profile, path: &Path) -> Result<Tree> {
    guard(profile, path)?;
    let mut tree = Tree::default();
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tree.kind = "absent".into();
            return Ok(tree);
        }
        Err(_) => {
            return Err(fail(
                "PERMISSION",
                "The proposed target cannot be inspected.",
            ));
        }
    };
    if is_link(&meta) {
        return Err(fail("UNSAFE_PATH", "A target link was rejected."));
    }
    if meta.is_file() {
        tree.kind = "file".into();
        tree.files
            .insert(String::new(), STANDARD.encode(read_regular(path)?));
        return Ok(tree);
    }
    if !meta.is_dir() {
        return Err(fail(
            "UNSAFE_FILE",
            "The target is not a file or directory.",
        ));
    }
    tree.kind = "directory".into();
    let mut pending = vec![path.to_path_buf()];
    let mut total = 0u64;
    let mut directories = 0;
    while let Some(dir) = pending.pop() {
        directories += 1;
        if directories > MAX_FILES {
            return Err(fail("LIMIT", "Directory inventory exceeds the limit."));
        }
        guard(profile, &dir)?;
        for entry in fs::read_dir(&dir)
            .map_err(|_| fail("PERMISSION", "A package directory cannot be read."))?
        {
            let p = entry
                .map_err(|_| fail("FILESYSTEM", "A directory entry cannot be read."))?
                .path();
            guard(profile, &p)?;
            let m = fs::symlink_metadata(&p)
                .map_err(|_| fail("FILESYSTEM", "An entry cannot be inspected."))?;
            if is_link(&m) {
                return Err(fail(
                    "UNSAFE_FILE",
                    "Package links and junctions are unsupported.",
                ));
            }
            if m.is_dir() {
                pending.push(p);
            } else if m.is_file() {
                if tree.files.len() >= MAX_FILES {
                    return Err(fail("LIMIT", "File count exceeds the limit."));
                }
                let rel = relative(
                    &p.strip_prefix(path)
                        .map_err(|_| fail("TARGET", "Package containment failed."))?
                        .to_string_lossy()
                        .replace('\\', "/"),
                )?;
                let bytes = read_regular(&p)?;
                total += bytes.len() as u64;
                if total > MAX_TOTAL {
                    return Err(fail("LIMIT", "Package size exceeds the limit."));
                }
                tree.files.insert(rel, STANDARD.encode(bytes));
            } else {
                return Err(fail("UNSAFE_FILE", "A special file was rejected."));
            }
        }
    }
    Ok(tree)
}
pub fn image(tree: &Tree) -> Result<Image> {
    let mut files = Vec::new();
    for (path, data) in &tree.files {
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| fail("INTEGRITY", "Invalid snapshot bytes."))?;
        files.push(FileInfo {
            path: path.clone(),
            sha256: hash(&bytes),
            size: bytes.len() as u64,
        });
    }
    let bytes = serde_json::to_vec(&(&tree.kind, &files))
        .map_err(|_| fail("INTEGRITY", "Invalid inventory."))?;
    Ok(Image {
        kind: tree.kind.clone(),
        fingerprint: hash(&bytes),
        files,
    })
}
pub fn file_tree(bytes: &[u8]) -> Tree {
    Tree {
        kind: "file".into(),
        files: [(String::new(), STANDARD.encode(bytes))].into(),
    }
}
pub fn missing() -> Tree {
    Tree {
        kind: "absent".into(),
        files: BTreeMap::new(),
    }
}
pub fn text(tree: &Tree) -> Result<String> {
    if tree.kind == "absent" {
        return Ok(String::new());
    }
    if tree.kind != "file" {
        return Err(fail(
            "CONFIG",
            "A configuration target is not an ordinary file.",
        ));
    }
    String::from_utf8(
        STANDARD
            .decode(
                tree.files
                    .get("")
                    .ok_or_else(|| fail("INTEGRITY", "Missing configuration bytes."))?,
            )
            .map_err(|_| fail("INTEGRITY", "Invalid configuration snapshot."))?,
    )
    .map_err(|_| fail("CONFIG", "Configuration must be UTF-8."))
}
pub fn write_new(profile: &Profile, path: &Path, tree: &Tree) -> Result<()> {
    guard(profile, path)?;
    if path.exists() {
        return Err(fail("CONFLICT", "A staging target unexpectedly exists."));
    }
    if tree.kind == "absent" {
        return Ok(());
    }
    if tree.kind == "directory" {
        fs::create_dir_all(path)
            .map_err(|_| fail("PERMISSION", "Staging directory creation failed."))?;
    }
    for (relative_path, encoded) in &tree.files {
        let target = if tree.kind == "file" {
            path.to_path_buf()
        } else {
            path.join(relative(relative_path)?)
        };
        guard(profile, &target)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| fail("PERMISSION", "The staging parent could not be created."))?;
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| fail("INTEGRITY", "Staging bytes are invalid."))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| fail("PERMISSION", "A staging file could not be created."))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| fail("FILESYSTEM", "A staging file could not be persisted."))?;
    }
    if image(&read(profile, path)?)? != image(tree)? {
        return Err(fail("INTEGRITY", "Staging verification failed."));
    }
    Ok(())
}
pub fn rename(profile: &Profile, source: &Path, target: &Path) -> Result<()> {
    guard(profile, source)?;
    guard(profile, target)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                a.as_ptr(),
                b.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(fail(
                "FILESYSTEM",
                "The same-volume file transition failed; recovery data was retained.",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target)
            .map_err(|_| fail("FILESYSTEM", "The same-volume transition failed."))
    }
}
pub fn remove_owned(profile: &Profile, path: &Path, expected: &Image) -> Result<()> {
    guard(profile, path)?;
    if image(&read(profile, path)?)? != *expected {
        return Err(fail(
            "LOCAL_MODIFIED",
            "Files changed; nothing was removed.",
        ));
    }
    if expected.kind == "directory" {
        fs::remove_dir_all(path)
    } else if expected.kind == "file" {
        fs::remove_file(path)
    } else {
        return Ok(());
    }
    .map_err(|_| fail("PERMISSION", "Owned files could not be removed."))
}
pub fn scratch(
    profile: &Profile,
    target: &Path,
    id: &str,
    index: usize,
    label: &str,
) -> Result<PathBuf> {
    let root = profile
        .approved_root(target)
        .ok_or_else(|| fail("TARGET", "No approved root for same-volume staging."))?;
    let path = root
        .join(".skillverse-transactions")
        .join(id)
        .join(format!("{label}-{index}"));
    guard(profile, &path)?;
    Ok(path)
}
pub fn durable(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| fail("STORAGE", "The transaction store is unavailable."))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            fail(
                "STORAGE",
                "A transaction artifact already exists or cannot be created.",
            )
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            fail(
                "STORAGE",
                "The transaction artifact could not be persisted.",
            )
        })
}
pub fn exclusive_lock(profile: &Profile) -> Result<File> {
    fs::create_dir_all(&profile.work_root)
        .map_err(|_| fail("STORAGE", "Transaction storage is unavailable."))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options
        .open(profile.work_root.join("execution.lock"))
        .map_err(|_| {
            fail(
                "BUSY",
                "Another process is changing this installation environment.",
            )
        })
}
pub fn space(path: &Path, needed: u64) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let existing = path
            .ancestors()
            .find(|p| p.is_dir())
            .ok_or_else(|| fail("DISK", "No target volume."))?;
        let wide: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0;
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
            || free < needed
        {
            return Err(fail(
                "DISK_SPACE",
                "Available disk space is below the operation's required reserve.",
            ));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, needed);
    }
    Ok(())
}
