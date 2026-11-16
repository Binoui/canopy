use std::io::{self, Read};

/// Read the entire request before replying, so closing cannot reset an unread POST body.
pub fn request(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return Ok(bytes);
            }
        }
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
}

#[test]
fn reads_fragmented_headers_and_body() {
    struct Fragments(std::io::Cursor<Vec<u8>>);
    impl Read for Fragments {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(&mut out[..1])
        }
    }
    let input = b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\nx=123";
    assert_eq!(
        request(&mut Fragments(std::io::Cursor::new(input.to_vec()))).unwrap(),
        input
    );
}
