//! DPAPI helpers: secrets are protected for the current Windows user.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
};

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = blob(plaintext);
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptProtectData: {e}"))?;

        let cipher = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
        Ok(cipher)
    }
}

pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = blob(ciphertext);
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptUnprotectData: {e}"))?;

        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let mut cleanup = plain.clone();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
        for b in cleanup.iter_mut() {
            *b = 0;
        }
        Ok(plain)
    }
}
