//! EXIF time extraction for images

use crate::error::{Error, Result};
use crate::time::datetime::parse_exif;
use crate::time::{ExtractedTime, TimeSource};
use chrono::{NaiveDateTime, Timelike};
use exif::{In, Reader, Tag};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tracing::trace;

/// EXIF tags to try for date extraction, in priority order
const DATE_TAGS: &[Tag] = &[
    Tag::DateTimeOriginal,  // When the original image was taken
    Tag::DateTimeDigitized, // When the image was digitized
    Tag::DateTime,          // File modification date/time
];

/// 与日期标签对应的亚秒（SubSec）标签，用于补充毫秒信息。
fn subsec_tag_for(tag: Tag) -> Option<Tag> {
    match tag {
        Tag::DateTimeOriginal => Some(Tag::SubSecTimeOriginal),
        Tag::DateTimeDigitized => Some(Tag::SubSecTimeDigitized),
        Tag::DateTime => Some(Tag::SubSecTime),
        _ => None,
    }
}

/// 把 SubSec 字符串（`"123"` / `"123456"` / `"12"` 等）转换为纳秒。
///
/// 数字按小数秒处理：不足 9 位右补零，超过 9 位截断。
fn subsec_to_nanos(value: &str) -> Option<u32> {
    let digits: String = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(9)
        .collect();
    if digits.is_empty() {
        return None;
    }

    let mut nanos = digits.parse::<u32>().ok()?;
    for _ in digits.len()..9 {
        nanos *= 10;
    }
    Some(nanos)
}

/// 把 SubSec 字段合并进基础日期时间；无有效亚秒时保持原值。
fn combine_subsec(datetime: NaiveDateTime, value: &str) -> NaiveDateTime {
    subsec_to_nanos(value)
        .and_then(|nanos| datetime.with_nanosecond(nanos))
        .unwrap_or(datetime)
}

/// Extract creation time from EXIF metadata
///
/// 返回的 [`ExtractedTime`] 会尽量携带 EXIF `SubSecTime*` 标签中的毫秒信息，
/// `has_millis` 表示元数据中是否显式含有亚秒。
pub fn extract_exif_time(path: &Path) -> Result<ExtractedTime> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let exif = Reader::new()
        .read_from_container(&mut reader)
        .map_err(|e| Error::ExifRead {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

    // Try each date tag in priority order
    for tag in DATE_TAGS {
        if let Some(field) = exif.get_field(*tag, In::PRIMARY)
            && let Some(datetime) = parse_exif(&field.display_value().to_string())
        {
            let subsec_tag = subsec_tag_for(*tag);
            let subsec_field = subsec_tag.and_then(|t| exif.get_field(t, In::PRIMARY));
            let has_millis = datetime.nanosecond() != 0 || subsec_field.is_some();
            let timestamp = subsec_field
                .map(|sub| combine_subsec(datetime, &sub.display_value().to_string()))
                .unwrap_or(datetime);

            trace!(?path, ?tag, "Found EXIF date");
            return Ok(ExtractedTime {
                timestamp,
                source: TimeSource::Exif,
                has_millis,
            });
        }
    }

    Err(Error::ExifRead {
        path: path.to_path_buf(),
        message: "No valid date tag found in EXIF data".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_exif() {
        // Standard EXIF format
        let dt = parse_exif("2024:01:15 14:30:00").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 0);

        // With quotes
        let dt = parse_exif("\"2024:01:15 14:30:00\"").unwrap();
        assert_eq!(dt.year(), 2024);

        // Alternative formats
        let dt = parse_exif("2024-01-15 14:30:00").unwrap();
        assert_eq!(dt.year(), 2024);

        // Invalid format
        assert!(parse_exif("invalid").is_none());
    }

    /// 构造带 DateTimeOriginal（可选 SubSecTimeOriginal）的最小 JPEG
    fn exif_jpeg(datetime: &str, subsec: Option<&str>) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00");

        let dt_len = (datetime.len() + 1) as u32;
        let sub_len = subsec.map(|s| (s.len() + 1) as u32).unwrap_or(0);
        // ASCII 值 ≤ 4 字节（含 NUL）时按 TIFF 规则内联在条目中，不写偏移
        let sub_inline = sub_len <= 4;
        let exif_ifd_offset = 0x1A;
        let entry_count: u16 = if subsec.is_some() { 2 } else { 1 };
        let dt_offset = exif_ifd_offset + 2 + entry_count as u32 * 12 + 4;
        let sub_offset = if sub_inline { 0 } else { dt_offset + dt_len };

        // IFD0：1 个 ExifIFDPointer 条目
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0x8769u16.to_le_bytes());
        payload.extend_from_slice(&4u16.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&exif_ifd_offset.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        // Exif 子 IFD
        payload.extend_from_slice(&entry_count.to_le_bytes());
        payload.extend_from_slice(&0x9003u16.to_le_bytes());
        payload.extend_from_slice(&2u16.to_le_bytes());
        payload.extend_from_slice(&dt_len.to_le_bytes());
        payload.extend_from_slice(&dt_offset.to_le_bytes());
        if let Some(subsec) = subsec {
            payload.extend_from_slice(&0x9291u16.to_le_bytes());
            payload.extend_from_slice(&2u16.to_le_bytes());
            payload.extend_from_slice(&sub_len.to_le_bytes());
            if sub_inline {
                let mut inline = [0u8; 4];
                inline[..subsec.len()].copy_from_slice(subsec.as_bytes());
                payload.extend_from_slice(&inline);
            } else {
                payload.extend_from_slice(&sub_offset.to_le_bytes());
            }
        }
        payload.extend_from_slice(&0u32.to_le_bytes());

        payload.extend_from_slice(datetime.as_bytes());
        payload.push(0);
        if !sub_inline && let Some(subsec) = subsec {
            payload.extend_from_slice(subsec.as_bytes());
            payload.push(0);
        }

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xD8]);
        bytes.extend_from_slice(&[0xFF, 0xE1]);
        bytes.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        bytes
    }

    #[test]
    fn test_extract_exif_time_with_subsec_millis() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        fs::write(&path, exif_jpeg("2024:01:15 14:30:00", Some("123"))).unwrap();

        let time = extract_exif_time(&path).unwrap();
        assert_eq!(time.source, TimeSource::Exif);
        assert!(time.has_millis);
        assert_eq!(
            time.timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            "2024-01-15 14:30:00.123"
        );
    }

    #[test]
    fn test_extract_exif_time_without_subsec_has_no_millis() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        fs::write(&path, exif_jpeg("2024:01:15 14:30:00", None)).unwrap();

        let time = extract_exif_time(&path).unwrap();
        assert_eq!(time.source, TimeSource::Exif);
        assert!(!time.has_millis);
        assert_eq!(time.timestamp.nanosecond(), 0);
    }

    #[test]
    fn test_subsec_to_nanos_pads_to_millis() {
        assert_eq!(subsec_to_nanos("123"), Some(123_000_000));
        assert_eq!(subsec_to_nanos("123456"), Some(123_456_000));
        assert_eq!(subsec_to_nanos("12"), Some(120_000_000));
        assert_eq!(subsec_to_nanos("000"), Some(0));
        assert_eq!(subsec_to_nanos("abc"), None);
    }
}
