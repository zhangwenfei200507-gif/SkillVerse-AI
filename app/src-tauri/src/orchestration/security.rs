use super::*;

pub(super) const REVIEWED_CODEX_SHA256: &str =
    "9cbcc600be53a3aa18c1d55b36b462ebd2844ac16e24b991654774b0819599d5";
pub(super) const FIRST_PARTY_CODEX_SHA256: &str =
    "4b3286e35d7fe866c6b0eb3f07d7a4cdb5e96489576170dfd6b3a15aa1c67382";
#[cfg(windows)]
const FIRST_PARTY_CODEX_PATH: &str = "C:/Program Files/WindowsApps/OpenAI.Codex_26.1103.4239.0_x64__2p2nqsd0c76g0/app/resources/codex.exe";
#[cfg(windows)]
const FIRST_PARTY_SIGNER_THUMBPRINT: &str = "de62726db667b59db0fd14b38f858d16c96804a0";

pub(super) struct CodexExecutable {
    path: std::path::PathBuf,
    #[cfg(windows)]
    _file: std::fs::File,
}

impl CodexExecutable {
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Recheck the selected binary at each launch. Keep its file handle open through
/// process creation so an updater cannot write or delete that file after hashing.
pub(super) fn codex_executable() -> Result<CodexExecutable> {
    let path = selected_codex_path()?;
    #[cfg(windows)]
    let file = open_verified_executable(&path, REVIEWED_CODEX_SHA256)?;
    Ok(CodexExecutable {
        path,
        #[cfg(windows)]
        _file: file,
    })
}

fn selected_codex_path() -> Result<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(if cfg!(windows) { "codex.exe" } else { "codex" }))
        .find(|path| path.is_file())
        .ok_or("CODEX_CLI_NOT_FOUND")?
        .canonicalize()
        .map_err(|_| "CODEX_EXECUTABLE_UNREADABLE".into())
}

pub(super) fn first_party_codex_executable() -> Result<CodexExecutable> {
    #[cfg(not(windows))]
    return Err("CODEX_FIRST_PARTY_WINDOWS_TRUST_REQUIRED".into());
    #[cfg(windows)]
    {
        let path = selected_codex_path()?;
        let approved = Path::new(FIRST_PARTY_CODEX_PATH)
            .canonicalize()
            .map_err(|_| "CODEX_FIRST_PARTY_STORE_PATH_UNAVAILABLE")?;
        if path != approved {
            return Err("CODEX_FIRST_PARTY_STORE_PATH_MISMATCH".into());
        }
        let mut file = open_verified_executable(&path, FIRST_PARTY_CODEX_SHA256)?;
        use std::io::Seek;
        file.rewind().map_err(|_| "CODEX_EXECUTABLE_UNREADABLE")?;
        verify_first_party_signature(&path, &file)?;
        Ok(CodexExecutable { path, _file: file })
    }
}

#[cfg(windows)]
fn verify_first_party_signature(path: &Path, file: &std::fs::File) -> Result<()> {
    use sha1::{Digest, Sha1};
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Security::WinTrust::*};
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: name.as_ptr(),
        hFile: file.as_raw_handle(),
        ..Default::default()
    };
    let mut request = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // WinTrust owns the returned chain until WTD_STATEACTION_CLOSE.
    let (status, signer_matches) = unsafe {
        let status = WinVerifyTrustEx(INVALID_HANDLE_VALUE, &mut action, &mut request);
        let signer_matches = if status == 0 {
            let provider = WTHelperProvDataFromStateData(request.hWVTStateData);
            (!provider.is_null())
                .then(|| WTHelperGetProvSignerFromChain(provider, 0, 0, 0))
                .filter(|signer| !signer.is_null())
                .and_then(|signer| WTHelperGetProvCertFromChain(signer, 0).as_ref())
                .and_then(|cert| cert.pCert.as_ref())
                .filter(|cert| {
                    !cert.pbCertEncoded.is_null() && (1..=65_536).contains(&cert.cbCertEncoded)
                })
                .is_some_and(|cert| {
                    let bytes =
                        std::slice::from_raw_parts(cert.pbCertEncoded, cert.cbCertEncoded as usize);
                    format!("{:x}", Sha1::digest(bytes)) == FIRST_PARTY_SIGNER_THUMBPRINT
                })
        } else {
            false
        };
        request.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrustEx(INVALID_HANDLE_VALUE, &mut action, &mut request);
        (status, signer_matches)
    };
    if status != 0 {
        return Err(format!(
            "CODEX_FIRST_PARTY_SIGNATURE_INVALID_{:08X}",
            status as u32
        ));
    }
    if !signer_matches {
        return Err("CODEX_FIRST_PARTY_SIGNER_MISMATCH".into());
    }
    Ok(())
}

#[cfg(windows)]
fn open_verified_executable(path: &Path, expected: &str) -> Result<std::fs::File> {
    use sha2::Digest;
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|_| "CODEX_EXECUTABLE_UNREADABLE")?;
    let mut hash = sha2::Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "CODEX_EXECUTABLE_UNREADABLE")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err("CODEX_RUNTIME_BINARY_CHANGED_REAUDIT_REQUIRED_NOT_VERIFIED".into());
    }
    Ok(file)
}

#[cfg(all(test, windows))]
#[test]
fn reviewed_executable_is_locked_and_rechecked_after_replacement() {
    let path = std::env::temp_dir().join(format!("skillverse-binary-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"reviewed bytes").unwrap();
    let expected = digest(b"reviewed bytes");
    let handle = open_verified_executable(&path, &expected).unwrap();
    assert!(std::fs::write(&path, b"changed bytes").is_err());
    drop(handle);
    std::fs::write(&path, b"changed bytes").unwrap();
    assert_eq!(
        open_verified_executable(&path, &expected).unwrap_err(),
        "CODEX_RUNTIME_BINARY_CHANGED_REAUDIT_REQUIRED_NOT_VERIFIED"
    );
    std::fs::remove_file(path).unwrap();
}
/// An owned process tree dies with SkillVerse, including on an OS-level exit.
pub(super) struct ProcessGuard {
    pub child: std::process::Child,
    #[cfg(windows)]
    job: usize,
}
impl ProcessGuard {
    pub fn new(mut child: std::process::Child) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{
                Foundation::CloseHandle,
                System::JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
            };
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("OWNED_PROCESS_JOB_REQUIRED".into());
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    std::mem::size_of_val(&info) as u32,
                ) == 0
                    || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0
                {
                    CloseHandle(job);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("OWNED_PROCESS_JOB_REQUIRED".into());
                }
                return Ok(Self {
                    child,
                    job: job as usize,
                });
            }
        }
        #[cfg(not(windows))]
        {
            Ok(Self { child })
        }
    }
}
impl std::ops::Deref for ProcessGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl std::ops::DerefMut for ProcessGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.child.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job as _);
        }
    }
}
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub fn ordinary(path: &Path) -> bool {
    path.ancestors().all(|p| {
        std::fs::symlink_metadata(p).is_ok_and(|m| {
            if m.file_type().is_symlink() {
                return false;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
            true
        })
    })
}
pub fn project(raw: &str) -> Result<PathBuf> {
    let p = PathBuf::from(raw);
    if !p.is_absolute() || !p.is_dir() || !ordinary(&p) {
        return Err("PROJECT_CONTEXT_REQUIRED_OR_LINKED".into());
    }
    std::fs::canonicalize(p).map_err(|_| "PROJECT_UNREADABLE".into())
}
pub fn root() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::var_os("SKILLVERSE_ORCHESTRATION_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir()
                    .join(format!("skillverse-execution-tests-{}", std::process::id()))
            });
    }
    #[cfg(not(test))]
    {
        if let Some(path) = crate::isolated_installation_root() {
            return path.join("orchestration");
        }
        PathBuf::from("D:/PersonalProject_DevTools/SkillVerseOrchestration")
    }
}
pub fn owned_dir(path: &Path) -> Result<()> {
    if !within_owned_root(path) {
        return Err("OUTSIDE_OWNED_WORKSPACE".into());
    }
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("WORKSPACE_PARENT")?
    }
    if !ordinary(ancestor) {
        return Err("LINKED_WORKSPACE".into());
    }
    std::fs::create_dir_all(path).map_err(|_| "WORKSPACE_CREATE")?;
    if !ordinary(path) {
        return Err("LINKED_WORKSPACE".into());
    }
    Ok(())
}
pub fn within_owned_root(path: &Path) -> bool {
    path.starts_with(root())
        && !path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
}
pub fn read(path: &Path, max: u64) -> Result<Vec<u8>> {
    if !ordinary(path) {
        return Err("LINKED_OR_MISSING_FILE".into());
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(1).custom_flags(0x00200000);
    }
    let file = opts.open(path).map_err(|_| "FILE_UNREADABLE")?;
    let meta = file.metadata().map_err(|_| "FILE_METADATA")?;
    if !meta.is_file() || meta.len() > max {
        return Err("FILE_SIZE_LIMIT".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut info = std::mem::MaybeUninit::<
            windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
        >::uninit();
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
                file.as_raw_handle() as _,
                info.as_mut_ptr(),
            )
        };
        if ok == 0 || unsafe { info.assume_init() }.nNumberOfLinks != 1 {
            return Err("LINKED_FILE".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "FILE_READ")?;
    if bytes.len() as u64 > max {
        return Err("FILE_SIZE_LIMIT".into());
    }
    Ok(bytes)
}
pub fn seal(path: &Path, bytes: &[u8]) -> Result<String> {
    owned_dir(path.parent().ok_or("EVIDENCE_PARENT")?)?;
    let hash = digest(bytes);
    if path.exists() {
        if digest(&read(path, 262144)?) == hash {
            return Ok(hash);
        }
        return Err("OWNED_FILE_CONFLICT_PRESERVED".into());
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0).custom_flags(0x00200000);
    }
    let mut file = opts.open(path).map_err(|_| "EVIDENCE_CREATE")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "EVIDENCE_WRITE")?;
    Ok(hash)
}
pub fn inspect(raw: &str) -> Result<Value> {
    let dir = project(raw)?;
    let mut files = Vec::new();
    for name in ["package.json", "README.md", "tsconfig.json"] {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        let bytes = read(&path, 131072)?;
        let mut summary = json!({"name":name,"bytes":bytes.len(),"sha256":digest(&bytes)});
        if name == "package.json" {
            let data: Value = serde_json::from_slice(&bytes).map_err(|_| "INVALID_PACKAGE_JSON")?;
            summary["packageName"] = json!(clean(data["name"].as_str().unwrap_or("UNKNOWN")));
            summary["scriptsAvailable"] = json!(
                data["scripts"]
                    .as_object()
                    .map(|s| s.keys().take(30).map(|k| clean(k)).collect::<Vec<_>>())
                    .unwrap_or_default()
            );
            summary["dependencies"] = json!(
                data["dependencies"]
                    .as_object()
                    .map(|s| s.keys().take(40).map(|k| clean(k)).collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        }
        files.push(summary)
    }
    Ok(
        json!({"scope":"EXPLICIT_PROJECT_METADATA_ONLY_NO_BUILD_OR_BROWSER_TEST","files":files,"executedScripts":false,"network":false}),
    )
}
