use super::types::ApiError;

pub trait Credentials: Send + Sync {
    fn read(&self) -> Result<Option<String>, ApiError>;
    fn save(&self, value: &str) -> Result<(), ApiError>;
    fn remove(&self) -> Result<(), ApiError>;
}
pub struct SystemCredentials {
    target: String,
}
impl SystemCredentials {
    pub(crate) fn intelligence_for(scope: &str) -> Self {
        Self {
            target: format!(
                "SkillVerse AI/CapabilityIntelligence/{}",
                super::types::digest(scope.as_bytes())
            ),
        }
    }
}
impl Default for SystemCredentials {
    fn default() -> Self {
        Self {
            target: "SkillVerse AI/GitHub/PublicDiscovery".into(),
        }
    }
}
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
#[cfg(windows)]
impl Credentials for SystemCredentials {
    fn read(&self) -> Result<Option<String>, ApiError> {
        use windows_sys::Win32::{
            Foundation::{ERROR_NOT_FOUND, GetLastError},
            Security::Credentials::*,
        };
        let target = wide(&self.target);
        let mut credential = std::ptr::null_mut();
        // The API owns this allocation; always release it with CredFree.
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
                return if GetLastError() == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err(ApiError::new(
                        "CREDENTIAL_STORE",
                        "Windows Credential Manager could not be read.",
                    ))
                };
            }
            let bytes = std::slice::from_raw_parts(
                (*credential).CredentialBlob,
                (*credential).CredentialBlobSize as usize,
            );
            let result = String::from_utf8(bytes.to_vec()).map(Some).map_err(|_| {
                ApiError::new("CREDENTIAL_STORE", "Stored GitHub credential is invalid.")
            });
            CredFree(credential.cast());
            result
        }
    }
    fn save(&self, value: &str) -> Result<(), ApiError> {
        use windows_sys::Win32::Security::Credentials::*;
        if value.len() < 8
            || value.len() > 4096
            || !value.is_ascii()
            || value.chars().any(char::is_whitespace)
        {
            return Err(ApiError::new(
                "INVALID_TOKEN",
                "Enter a valid GitHub token without whitespace.",
            ));
        }
        let mut target = wide(&self.target);
        let mut username = wide("GitHub public read-only discovery");
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: value.len() as u32,
            CredentialBlob: value.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(ApiError::new(
                "CREDENTIAL_STORE",
                "Windows Credential Manager did not save the token.",
            ));
        }
        Ok(())
    }
    fn remove(&self) -> Result<(), ApiError> {
        use windows_sys::Win32::{
            Foundation::{ERROR_NOT_FOUND, GetLastError},
            Security::Credentials::*,
        };
        let target = wide(&self.target);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
            && unsafe { GetLastError() } != ERROR_NOT_FOUND
        {
            return Err(ApiError::new(
                "CREDENTIAL_STORE",
                "Windows Credential Manager did not remove the token.",
            ));
        }
        Ok(())
    }
}
#[cfg(not(windows))]
impl Credentials for SystemCredentials {
    fn read(&self) -> Result<Option<String>, ApiError> {
        Ok(None)
    }
    fn save(&self, _: &str) -> Result<(), ApiError> {
        Err(ApiError::new(
            "CREDENTIAL_STORE",
            "Secure token storage is currently implemented for Windows only.",
        ))
    }
    fn remove(&self) -> Result<(), ApiError> {
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn windows_credential_roundtrip_uses_an_isolated_test_target() {
        let store = SystemCredentials {
            target: format!("SkillVerse AI/TestOnly/{}", uuid::Uuid::new_v4()),
        };
        assert!(store.read().unwrap().is_none());
        store.save("synthetic-test-value-no-real-access").unwrap();
        let reopened = SystemCredentials {
            target: store.target.clone(),
        };
        let result = reopened.read();
        let replaced = reopened.save("replacement-test-value-no-real-access");
        let replacement = reopened.read();
        let removed = store.remove();
        assert_eq!(
            result.unwrap().as_deref(),
            Some("synthetic-test-value-no-real-access")
        );
        replaced.unwrap();
        assert_eq!(
            replacement.unwrap().as_deref(),
            Some("replacement-test-value-no-real-access")
        );
        removed.unwrap();
        assert!(store.read().unwrap().is_none());
    }
}
