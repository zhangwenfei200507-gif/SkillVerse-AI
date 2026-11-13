use super::{
    files::{Tree, durable, image},
    types::*,
};
use std::path::Path;
#[cfg(windows)]
fn transform(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| fail("BACKUP", "Snapshot is too large."))?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let success = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(fail(
            "BACKUP",
            "Windows could not protect or restore this user-bound snapshot.",
        ));
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut _);
    }
    Ok(result)
}
#[cfg(not(windows))]
fn transform(_bytes: &[u8], _decrypt: bool) -> Result<Vec<u8>> {
    Err(fail(
        "PLATFORM",
        "Secure installer backups currently require Windows DPAPI.",
    ))
}
pub fn save(path: &Path, tree: &Tree) -> Result<()> {
    let bytes =
        serde_json::to_vec(tree).map_err(|_| fail("BACKUP", "Snapshot could not be encoded."))?;
    durable(path, &transform(&bytes, false)?)?;
    if image(&load(path)?)? != image(tree)? {
        return Err(fail(
            "BACKUP",
            "The encrypted snapshot could not be verified.",
        ));
    }
    Ok(())
}
pub fn load(path: &Path) -> Result<Tree> {
    let bytes = std::fs::read(path)
        .map_err(|_| fail("BACKUP", "A required recovery snapshot is unavailable."))?;
    if bytes.len() > 100 * 1024 * 1024 {
        return Err(fail("BACKUP", "Snapshot exceeds its size limit."));
    }
    serde_json::from_slice(&transform(&bytes, true)?)
        .map_err(|_| fail("BACKUP", "Snapshot contents are invalid."))
}
