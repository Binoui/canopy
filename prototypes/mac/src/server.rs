//! A tiny local site for the prototype: numbered tall pages that link to each other, and a video
//! page. It counts requests per path so the test can tell whether a restore fetched a page again.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const CLIP: &[u8] = include_bytes!("../assets/clip.mp4");

#[derive(Clone)]
pub struct Site {
    pub base: String,
    hits: Arc<Mutex<HashMap<String, u32>>>,
}

impl Site {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        let hits = Arc::new(Mutex::new(HashMap::new()));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let counted = counted.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &counted);
                });
            }
        });
        Ok(Self { base, hits })
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub fn hits(&self, path: &str) -> u32 {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

fn serve(stream: TcpStream, hits: &Mutex<HashMap<String, u32>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut stream = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(());
        }
        let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let mut range = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
                break;
            }
            let lower = header.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("range:") {
                range = parse_range(value.trim());
            }
        }
        *hits.lock().unwrap().entry(path.clone()).or_insert(0) += 1;

        if path == "/clip.mp4" {
            let total = CLIP.len();
            let (start, end) = match range {
                Some((start, end)) => (start.min(total - 1), end.unwrap_or(total - 1).min(total - 1)),
                None => (0, total - 1),
            };
            let body = &CLIP[start..=end];
            let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\n\
                 Content-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
        } else {
            let (status, body) = page(&path);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Cache-Control: no-store\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body.as_bytes())?;
        }
        stream.flush()?;
    }
}

fn parse_range(value: &str) -> Option<(usize, Option<usize>)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    Some((start.trim().parse().ok()?, end.trim().parse().ok()))
}

/// Every page counts clicks, animation frames and timer ticks in `window.__proto`, so the test can
/// ask a page what it saw.
const COUNTERS: &str = r#"<script>
window.__proto = { clicks: 0, frames: 0, ticks: 0 };
addEventListener('click', () => { window.__proto.clicks++; });
(function frame() { window.__proto.frames++; requestAnimationFrame(frame); })();
setInterval(() => { window.__proto.ticks++; }, 100);
</script>"#;

fn page(path: &str) -> (&'static str, String) {
    if let Some(n) = path.strip_prefix("/p/").and_then(|n| n.parse::<u32>().ok()) {
        let hue = (n * 67) % 360;
        let body = format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><title>Page {n}</title>
<style>
  body {{ margin: 0; font: 16px -apple-system, sans-serif; color: #10210f;
         background: linear-gradient(hsl({hue} 60% 86%), hsl({hue} 60% 62%)); height: 4000px; }}
  main {{ padding: 80px 40px; }}
  h1 {{ font-size: 64px; margin: 0 0 16px; }}
  a {{ font-size: 24px; }}
  input {{ font-size: 20px; padding: 6px 10px; margin-top: 24px; display: block; }}
  .mark {{ position: absolute; left: 40px; font-size: 28px; opacity: .6; }}
</style></head><body><main>
  <h1>Page {n}</h1>
  <a id="next" href="/p/{next}">Go to page {next}</a>
  <input id="note" placeholder="type here, then checkpoint">
</main>
<div class="mark" style="top:1200px">1200 px down</div>
<div class="mark" style="top:2400px">2400 px down</div>
{COUNTERS}</body></html>"#,
            next = n + 1
        );
        return ("200 OK", body);
    }
    if path == "/video" {
        let body = format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><title>Video</title>
<style>body {{ margin: 0; background: #111; color: #ddd; font: 16px -apple-system, sans-serif;
  display: grid; place-items: center; height: 100vh; }} video {{ width: 640px; }}</style></head>
<body><div><video id="v" src="/clip.mp4" autoplay loop playsinline controls></video>
<p>Test clip with a 440 Hz tone.</p></div>{COUNTERS}</body></html>"#
        );
        return ("200 OK", body);
    }
    ("404 Not Found", "<!doctype html><title>Not found</title>Not found".to_string())
}
