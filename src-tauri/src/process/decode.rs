#[cfg(target_os = "windows")]
use windows_sys::Win32::Globalization::{GetOEMCP, MultiByteToWideChar, MB_ERR_INVALID_CHARS};

/// Decodes process output bytes, preferring UTF-8 and falling back, on Windows only,
/// to the console's OEM code page — output written before a script switches the
/// console encoding (e.g. a PowerShell profile) still arrives in that legacy page.
#[cfg(target_os = "windows")]
pub(crate) fn decode_output_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) if has_utf8_multibyte_sequence(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Err(_) => decode_oem(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// A lead byte with the right run of continuation bytes means the line is UTF-8
/// with a few corrupted bytes, not a whole line in a legacy code page — lossy
/// UTF-8 decoding degrades that case better than reinterpreting it as OEM.
#[cfg(target_os = "windows")]
fn has_utf8_multibyte_sequence(bytes: &[u8]) -> bool {
    for (i, &lead) in bytes.iter().enumerate() {
        let extra = match lead {
            0xC2..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF4 => 3,
            _ => continue,
        };
        if i + extra < bytes.len()
            && bytes[i + 1..=i + extra]
                .iter()
                .all(|&b| (0x80..=0xBF).contains(&b))
        {
            return true;
        }
    }
    false
}

/// cmd and pwsh both write redirected output in the console's OEM page, never
/// the ANSI page (measured: pwsh's French "chemin d'accès" and cmd's "chemin
/// réseau" errors both decode correctly only under CP850/437, not CP1252).
/// DBCS OEM pages (932/936/949/950) can reject bytes, so the fallback stays.
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
    fn decode_with_code_page_honours_the_page_it_is_given() {
        use super::decode_with_code_page;
        // "exécutable" with é encoded as 0x82 in CP850 and 0xE9 in CP1252.
        let cp850_bytes = [b'e', b'x', 0x82, b'c', b'u', b't', b'a', b'b', b'l', b'e'];
        let cp1252_bytes = [b'e', b'x', 0xE9, b'c', b'u', b't', b'a', b'b', b'l', b'e'];
        assert_eq!(
            decode_with_code_page(&cp850_bytes, 850).unwrap(),
            "ex\u{e9}cutable"
        );
        assert_eq!(
            decode_with_code_page(&cp1252_bytes, 1252).unwrap(),
            "ex\u{e9}cutable"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn has_utf8_multibyte_sequence_detects_a_lead_and_continuation_byte() {
        use super::has_utf8_multibyte_sequence;
        assert!(has_utf8_multibyte_sequence(&[0xC3, 0xBC]));
        assert!(!has_utf8_multibyte_sequence(&[0x8A, 0xAE, 0xAF, 0xFF]));
        assert!(!has_utf8_multibyte_sequence(&[0x82]));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn mostly_utf8_line_with_one_stray_byte_keeps_its_valid_text() {
        let mut bytes = "Müller ".as_bytes().to_vec();
        bytes.push(0xFF);
        assert_eq!(decode_output_bytes(&bytes), "Müller \u{fffd}");
    }
}
