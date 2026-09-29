use std::io::{self, BufRead, Write};
pub(crate) const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn read_frame(reader: &mut impl BufRead, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let count =
        std::io::Read::take(&mut *reader, (limit + 1) as u64).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if bytes.last() != Some(&b'\n') {
        return Err(io::Error::other(if count > limit {
            "request message limit exceeded"
        } else {
            "incomplete request at end of input"
        }));
    }
    bytes.pop();
    if bytes.len() > limit {
        return Err(io::Error::other("request message limit exceeded"));
    }
    Ok(Some(bytes))
}

pub(crate) struct LimitedFrame {
    pub bytes: Vec<u8>,
    pub limit: usize,
}
impl Write for LimitedFrame {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::other("event message limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_unterminated_frames_and_accepts_boundary() {
        assert!(read_frame(&mut &b"123456789"[..], 4).is_err());
        assert!(read_frame(&mut &b"123"[..], 4).is_err());
        assert_eq!(
            read_frame(&mut &b"1234\n"[..], 4).unwrap().unwrap(),
            b"1234"
        );
        let mut reader = &b"a\nb\n"[..];
        assert_eq!(read_frame(&mut reader, 4).unwrap().unwrap(), b"a");
        assert_eq!(read_frame(&mut reader, 4).unwrap().unwrap(), b"b");
    }
    #[test]
    fn serialization_cannot_grow_past_limit() {
        let mut frame = LimitedFrame {
            bytes: vec![],
            limit: 4,
        };
        assert!(serde_json::to_writer(&mut frame, &"a long value").is_err());
        assert!(frame.bytes.len() <= 4);
    }
}
