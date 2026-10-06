fn sqlite_size_header(encoded_page_size: u16, page_count: u32) -> [u8; 100] {
    let mut header = [0; 100];
    header[..16].copy_from_slice(b"SQLite format 3\0");
    header[16..18].copy_from_slice(&encoded_page_size.to_be_bytes());
    header[24..28].copy_from_slice(&7u32.to_be_bytes());
    header[28..32].copy_from_slice(&page_count.to_be_bytes());
    header[92..96].copy_from_slice(&7u32.to_be_bytes());
    header
}

#[test]
fn retention_archive_size_preserves_gzip_wrap_boundary() {
    let size = crate::maintenance::archive_sqlite_size_from_header;
    assert_eq!(
        size(&sqlite_size_header(4096, 1 << 20)).unwrap(),
        1u64 << 32
    );
    assert_eq!(
        size(&sqlite_size_header(4096, (1 << 20) + 2)).unwrap(),
        (1u64 << 32) + 8192,
    );
    assert_eq!(
        size(&sqlite_size_header(1, 65_537)).unwrap(),
        (1u64 << 32) + 65_536,
    );
}

#[test]
fn retention_archive_size_rejects_unusable_sqlite_metadata() {
    let size = crate::maintenance::archive_sqlite_size_from_header;
    assert!(size(&sqlite_size_header(4096, 0)).is_err());
    assert!(size(&sqlite_size_header(513, 2)).is_err());
    let mut header = sqlite_size_header(4096, 2);
    header[92..96].copy_from_slice(&8u32.to_be_bytes());
    assert!(size(&header).is_err());
    header = sqlite_size_header(4096, 2);
    header[0] = 0;
    assert!(size(&header).is_err());
}
