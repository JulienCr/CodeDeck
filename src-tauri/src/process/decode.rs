#[cfg(target_os = "windows")]
use windows_sys::Win32::Globalization::{GetOEMCP, MultiByteToWideChar, MB_ERR_INVALID_CHARS};

/// Decodes process output bytes, preferring UTF-8 and falling back, on Windows only,
/// to the console's OEM code page — output written before a script switches the
/// console encoding (e.g. a PowerShell profile) still arrives in that legacy page.
#[cfg(target_os = "windows")]
pub(crate) fn decode_output_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => decode_oem(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

#[cfg(target_os = "windows")]
fn decode_oem(bytes: &[u8]) -> Option<String> {
    decode_with_code_page(bytes, unsafe { GetOEMCP() })
}

#[cfg(target_os = "windows")]
fn decode_with_code_page(bytes: &[u8], code_page: u32) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    unsafe {
        let wide_len = MultiByteToWideChar(
            code_page,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        );
        if wide_len <= 0 {
            return None;
        }
        let mut wide = vec![0u16; wide_len as usize];
        let written = MultiByteToWideChar(
            code_page,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            wide.len() as i32,
        );
        if written <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&wide[..written as usize]))
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn decode_output_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::decode_output_bytes;

    #[test]
    fn valid_utf8_with_multibyte_chars_passes_through_unchanged() {
        assert_eq!(decode_output_bytes("Müller äöü".as_bytes()), "Müller äöü");
    }

    #[test]
    fn empty_input_decodes_to_empty_string() {
        assert_eq!(decode_output_bytes(&[]), "");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn cp850_bytes_decode_to_the_matching_accented_text() {
        use super::decode_with_code_page;
        // "exécutable" with é encoded as 0x82 in CP850 (French Windows OEM page).
        let bytes = [b'e', b'x', 0x82, b'c', b'u', b't', b'a', b'b', b'l', b'e'];
        assert_eq!(
            decode_with_code_page(&bytes, 850).unwrap(),
            "ex\u{e9}cutable"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn cp1252_bytes_decode_to_the_matching_accented_text() {
        use super::decode_with_code_page;
        // "exécutable" with é encoded as 0xE9 in CP1252 (Windows ANSI page).
        let bytes = [b'e', b'x', 0xE9, b'c', b'u', b't', b'a', b'b', b'l', b'e'];
        assert_eq!(
            decode_with_code_page(&bytes, 1252).unwrap(),
            "ex\u{e9}cutable"
        );
    }
}
