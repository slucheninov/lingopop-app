//! File helpers shared by the stores. Task 6 extends this module.

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Removes a leading UTF-8 BOM (Notepad adds one when the user edits JSON).
/// This is the single BOM rule of the crate; a BOM anywhere else is kept.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bom_removes_only_a_leading_bom() {
        assert_eq!(strip_bom(&[0xEF, 0xBB, 0xBF, b'{', b'}']), b"{}");
        assert_eq!(strip_bom(b"{}"), b"{}");
        let middle = [b'{', 0xEF, 0xBB, 0xBF, b'}'];
        assert_eq!(strip_bom(&middle), &middle);
        assert_eq!(strip_bom(&[]), b"");
        assert_eq!(strip_bom(&[0xEF, 0xBB]), &[0xEF, 0xBB]);
        assert_eq!(strip_bom(&[0xEF, 0xBB, 0xBF]), b"");
    }
}
