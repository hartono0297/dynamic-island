//! DPAPI in current-user scope, shared by the Google token file and the
//! clipboard history: only this Windows account can decrypt what it seals.
//! Decrypted plaintext is wiped in DPAPI's buffer before it is freed.

use std::io;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;

/// Seals `plain`; `description` is stored (unencrypted) with the blob.
pub fn protect(plain: &[u8], description: PCWSTR) -> io::Result<Vec<u8>> {
    let input = blob(plain)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            description,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(io::Error::other)?;
    Ok(unsafe { take_blob(output, false) })
}

/// Opens a blob `protect` sealed for this account; anything else fails.
pub fn unprotect(sealed: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob(sealed)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(io::Error::other)?;
    Ok(unsafe { take_blob(output, true) })
}

fn blob(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(data.len()).map_err(io::Error::other)?;
    // DPAPI only reads the input blob; `*mut` is just the C struct's field type.
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: data.as_ptr().cast_mut(),
    })
}

/// Copies a DPAPI output blob and frees it with `LocalFree`, zeroing it first
/// when it holds plaintext.
///
/// # Safety
/// `blob` must come from a successful `CryptProtectData`/`CryptUnprotectData`.
unsafe fn take_blob(blob: CRYPT_INTEGER_BLOB, wipe: bool) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    let len = blob.cbData as usize;
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, len) }.to_vec();
    unsafe {
        if wipe {
            for i in 0..len {
                std::ptr::write_volatile(blob.pbData.add(i), 0);
            }
        }
        LocalFree(Some(HLOCAL(blob.pbData.cast())));
    }
    bytes
}

/// Overwrites `bytes` with zeros in a way the optimizer keeps.
pub fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;

    #[test]
    fn round_trip_and_garbage_is_refused() {
        let sealed = protect(b"secret text", w!("Vale test")).unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert_eq!(unprotect(&sealed).unwrap(), b"secret text");
        assert!(unprotect(b"not a dpapi blob").is_err());
    }

    #[test]
    fn wipe_zeroes() {
        let mut v = b"abc".to_vec();
        wipe(&mut v);
        assert_eq!(v, [0, 0, 0]);
    }
}
