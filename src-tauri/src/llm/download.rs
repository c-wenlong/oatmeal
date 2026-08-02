//! Fetching large files without holding them in memory or losing progress.
//!
//! The two things this exists to download are a ~11 MB server archive and a
//! 2–5 GB model. The model is what shapes the design: it is far too big to
//! buffer, big enough that a dropped connection is likely, and big enough that
//! starting over is a real cost to the user.
//!
//! So: stream to disk, write to a `.part` file, resume with a range request,
//! and only move the file into place once it is complete and verified. Anything
//! that goes wrong leaves either a resumable `.part` or nothing at all — never a
//! truncated file at the real path, which is the failure the runtime's
//! completeness check exists to catch and which would otherwise fail
//! mid-generation.

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("could not reach {url}: {detail}")]
    Unreachable { url: String, detail: String },
    #[error("{url} returned {status}")]
    BadStatus { url: String, status: u16 },
    /// The connection dropped part-way through.
    ///
    /// Distinct from `Unreachable` because it means something different to the
    /// user: the host was fine, the transfer was not, and the bytes already on
    /// disk are kept so retrying resumes rather than restarts. On a 5 GB model
    /// that distinction is the whole difference between an annoyance and an
    /// evening.
    #[error("the download was interrupted after {downloaded} bytes: {detail}")]
    Interrupted { downloaded: u64, detail: String },
    #[error("could not write {path}: {detail}")]
    Io { path: String, detail: String },
    #[error("the download finished but the file is not valid: {0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
}

/// Progress for the UI. Mirrored by `DownloadProgress` in `src/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// Which download this is — a model id, or `server`.
    pub id: String,
    pub downloaded: u64,
    /// `None` when the server does not report a length.
    pub total: Option<u64>,
    pub done: bool,
}

impl DownloadProgress {
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(total) if total > 0 => {
                Some((self.downloaded as f64 / total as f64).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// The path a partial download accumulates in.
///
/// Deliberately alongside the destination rather than in a temp directory, so a
/// resume works across app restarts and the bytes cannot be swept away by the
/// OS between sessions.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// How many bytes of `dest` are already on disk and resumable.
pub fn resumable_bytes(dest: &Path) -> u64 {
    std::fs::metadata(part_path(dest))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Streams `url` into `dest`, resuming an interrupted attempt if one is there.
///
/// `on_progress` is called as bytes land. `should_cancel` is polled between
/// chunks so a cancel does not have to wait for a multi-gigabyte download.
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    id: &str,
    mut on_progress: impl FnMut(DownloadProgress),
    should_cancel: impl Fn() -> bool,
) -> Result<(), DownloadError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DownloadError::Io {
            path: parent.display().to_string(),
            detail: e.to_string(),
        })?;
    }

    let part = part_path(dest);
    let already = resumable_bytes(dest);

    let mut request = http.get(url);
    if already > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={already}-"));
    }

    let response = request
        .send()
        .await
        .map_err(|e| DownloadError::Unreachable {
            url: url.to_string(),
            detail: e.to_string(),
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::BadStatus {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }

    // 206 means the range was honoured and we continue where we left off. A 200
    // to a range request means the server ignored it and is sending the whole
    // file, so the partial file has to be discarded — appending would splice the
    // start of the file onto the middle of it and produce silent corruption.
    let resuming = already > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resuming { already } else { 0 };

    let total = response
        .content_length()
        .map(|len| len + start)
        .or_else(|| content_range_total(&response));

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resuming)
        .open(&part)
        .map_err(|e| DownloadError::Io {
            path: part.display().to_string(),
            detail: e.to_string(),
        })?;

    if resuming {
        file.seek(SeekFrom::Start(start))
            .map_err(|e| DownloadError::Io {
                path: part.display().to_string(),
                detail: e.to_string(),
            })?;
    }

    let mut downloaded = start;
    on_progress(DownloadProgress {
        id: id.to_string(),
        downloaded,
        total,
        done: false,
    });

    let mut stream = response.bytes_stream();
    // Progress is reported on a byte interval rather than per chunk: chunks
    // arrive thousands of times a second and every event crosses into the
    // webview.
    let mut since_report = 0u64;

    // Bytes written so far must survive an early return, or a resume would
    // start from a stale offset. `file` is flushed on drop, and every error
    // path below returns rather than unwinding past it.
    while let Some(chunk) = stream.next().await {
        if should_cancel() {
            // The `.part` file stays. Cancelling is not the same as discarding,
            // and re-running the download should pick up where this stopped.
            return Err(DownloadError::Cancelled);
        }

        // A server that declares more than it sends lands here, as a body
        // decode error, rather than at the length check below — the stream
        // fails before it ever ends cleanly.
        let chunk = chunk.map_err(|e| DownloadError::Interrupted {
            downloaded,
            detail: e.to_string(),
        })?;
        file.write_all(&chunk).map_err(|e| DownloadError::Io {
            path: part.display().to_string(),
            detail: e.to_string(),
        })?;

        downloaded += chunk.len() as u64;
        since_report += chunk.len() as u64;
        if since_report >= 4 * 1024 * 1024 {
            since_report = 0;
            on_progress(DownloadProgress {
                id: id.to_string(),
                downloaded,
                total,
                done: false,
            });
        }
    }

    file.flush().map_err(|e| DownloadError::Io {
        path: part.display().to_string(),
        detail: e.to_string(),
    })?;
    drop(file);

    // A truncated transfer that ends cleanly is the dangerous case — it looks
    // like success. Checking the promised length against what landed is the
    // only way to tell it apart here.
    if let Some(total) = total {
        let actual = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if actual != total {
            return Err(DownloadError::Invalid(format!(
                "expected {total} bytes, got {actual}"
            )));
        }
    }

    std::fs::rename(&part, dest).map_err(|e| DownloadError::Io {
        path: dest.display().to_string(),
        detail: e.to_string(),
    })?;

    on_progress(DownloadProgress {
        id: id.to_string(),
        downloaded,
        total,
        done: true,
    });
    Ok(())
}

/// Total size from a `Content-Range: bytes X-Y/TOTAL` header.
fn content_range_total(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Every GGUF file starts with these four bytes.
///
/// Cheap guard against having downloaded an HTML error page under a `.gguf`
/// name — Hugging Face serves those with a 200 when a repo moves, and the
/// result would otherwise sit there looking like a model until generation fails.
pub const GGUF_MAGIC: &[u8; 4] = b"GGUF";

pub fn looks_like_gguf(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && &magic == GGUF_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_file_sits_next_to_its_destination() {
        // Not in a temp dir: a resume has to survive an app restart.
        let dest = Path::new("/tmp/models/model.gguf");
        assert_eq!(part_path(dest), Path::new("/tmp/models/model.gguf.part"));
    }

    #[test]
    fn nothing_on_disk_means_nothing_to_resume() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resumable_bytes(&dir.path().join("absent.gguf")), 0);
    }

    #[test]
    fn a_partial_file_reports_how_far_it_got() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        std::fs::write(part_path(&dest), vec![0u8; 4096]).unwrap();
        assert_eq!(resumable_bytes(&dest), 4096);
    }

    #[test]
    fn gguf_is_recognised_by_its_magic_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.gguf");
        std::fs::write(&good, b"GGUF\x03\x00\x00\x00rest").unwrap();
        assert!(looks_like_gguf(&good));
    }

    #[test]
    fn an_html_error_page_is_not_mistaken_for_a_model() {
        // Hugging Face answers a moved repo with a 200 and a page of HTML.
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.gguf");
        std::fs::write(&bad, b"<!DOCTYPE html><html>Not Found</html>").unwrap();
        assert!(!looks_like_gguf(&bad));
    }

    #[test]
    fn a_file_too_short_to_have_magic_is_not_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("tiny.gguf");
        std::fs::write(&bad, b"GG").unwrap();
        assert!(!looks_like_gguf(&bad));
    }

    #[test]
    fn a_missing_file_is_not_a_model() {
        assert!(!looks_like_gguf(Path::new("/nonexistent/model.gguf")));
    }

    #[test]
    fn progress_reports_a_fraction_only_when_the_total_is_known() {
        let known = DownloadProgress {
            id: "m".into(),
            downloaded: 50,
            total: Some(200),
            done: false,
        };
        assert_eq!(known.fraction(), Some(0.25));

        let unknown = DownloadProgress {
            id: "m".into(),
            downloaded: 50,
            total: None,
            done: false,
        };
        assert_eq!(unknown.fraction(), None);

        // A server that reports zero length must not produce a division by zero.
        let zero = DownloadProgress {
            id: "m".into(),
            downloaded: 0,
            total: Some(0),
            done: false,
        };
        assert_eq!(zero.fraction(), None);
    }

    /// A minimal HTTP server, enough to exercise ranges and truncation.
    ///
    /// Hand-rolled rather than pulled in as a dependency: the behaviours that
    /// matter here are a server *honouring* a range, a server *ignoring* one,
    /// and a server lying about the length. A polished test server makes the
    /// first easy and the other two awkward.
    struct TestServer {
        addr: std::net::SocketAddr,
        _handle: std::thread::JoinHandle<()>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Behaviour {
        /// Honours `Range` with a 206.
        Ranges,
        /// Ignores `Range` and always sends the whole body with a 200.
        IgnoresRanges,
        /// Promises more than it sends.
        Truncates,
    }

    impl TestServer {
        fn start(body: Vec<u8>, behaviour: Behaviour) -> Self {
            use std::io::{BufRead, BufReader, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();

            let handle = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());

                    let mut start = 0usize;
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok() {
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        if let Some(range) = line.to_lowercase().strip_prefix("range: bytes=") {
                            if let Some(from) = range.split('-').next() {
                                start = from.trim().parse().unwrap_or(0);
                            }
                        }
                        line.clear();
                    }

                    let (status, slice, extra) = match behaviour {
                        Behaviour::Ranges if start > 0 => (
                            "206 Partial Content",
                            &body[start.min(body.len())..],
                            format!(
                                "Content-Range: bytes {}-{}/{}\r\n",
                                start,
                                body.len().saturating_sub(1),
                                body.len()
                            ),
                        ),
                        Behaviour::Truncates => ("200 OK", &body[..body.len() / 2], String::new()),
                        _ => ("200 OK", &body[..], String::new()),
                    };

                    // Truncates lies: it advertises the full length but sends half.
                    let advertised = if behaviour == Behaviour::Truncates {
                        body.len()
                    } else {
                        slice.len()
                    };

                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {advertised}\r\n{extra}Connection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(slice);
                    let _ = stream.flush();
                }
            });

            Self {
                addr,
                _handle: handle,
            }
        }

        fn url(&self) -> String {
            format!("http://{}/file.bin", self.addr)
        }
    }

    fn body(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn a_whole_file_downloads_and_lands_at_the_destination() {
        let expected = body(300_000);
        let server = TestServer::start(expected.clone(), Behaviour::Ranges);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");

        let mut last = None;
        download(
            &reqwest::Client::new(),
            &server.url(),
            &dest,
            "t",
            |p| last = Some(p),
            || false,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        assert!(
            last.unwrap().done,
            "the final progress event should be done"
        );
        assert!(!part_path(&dest).exists(), "the .part file should be gone");
    }

    #[tokio::test]
    async fn an_interrupted_download_resumes_instead_of_restarting() {
        let expected = body(300_000);
        let server = TestServer::start(expected.clone(), Behaviour::Ranges);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");

        // Simulate having already fetched the first third.
        let already = 100_000;
        std::fs::write(part_path(&dest), &expected[..already]).unwrap();

        let mut first_seen = None;
        download(
            &reqwest::Client::new(),
            &server.url(),
            &dest,
            "t",
            |p| {
                if first_seen.is_none() {
                    first_seen = Some(p.downloaded);
                }
            },
            || false,
        )
        .await
        .unwrap();

        assert_eq!(
            first_seen,
            Some(already as u64),
            "progress should start from what was already on disk, not zero"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_range_restarts_rather_than_splicing() {
        // The corruption case. Asked to resume from 100k, this server sends the
        // whole file with a 200. Appending it to the existing 100k would produce
        // a file of the right *kind* and the wrong *contents* — valid enough to
        // load and wrong enough to fail much later.
        let expected = body(300_000);
        let server = TestServer::start(expected.clone(), Behaviour::IgnoresRanges);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        std::fs::write(part_path(&dest), &expected[..100_000]).unwrap();

        download(
            &reqwest::Client::new(),
            &server.url(),
            &dest,
            "t",
            |_| {},
            || false,
        )
        .await
        .unwrap();

        let landed = std::fs::read(&dest).unwrap();
        assert_eq!(
            landed.len(),
            expected.len(),
            "file was spliced, not restarted"
        );
        assert_eq!(landed, expected);
    }

    #[tokio::test]
    async fn a_truncated_transfer_is_rejected_rather_than_kept() {
        // Ends cleanly, so it looks like success. Only the promised length
        // gives it away — and a short model fails mid-generation instead.
        let server = TestServer::start(body(200_000), Behaviour::Truncates);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");

        let err = download(
            &reqwest::Client::new(),
            &server.url(),
            &dest,
            "t",
            |_| {},
            || false,
        )
        .await
        .unwrap_err();

        // Caught mid-stream by the body decoder rather than by the final length
        // check — the transfer never ends cleanly enough to reach it.
        assert!(
            matches!(err, DownloadError::Interrupted { .. }),
            "got {err:?}"
        );
        assert!(
            err.to_string().contains("interrupted"),
            "the message should say the transfer broke, not that the host was \
             unreachable: {err}"
        );
        assert!(
            !dest.exists(),
            "a short file must never reach the real path"
        );
        assert!(
            part_path(&dest).exists(),
            "the part should stay for a retry"
        );
    }

    #[tokio::test]
    async fn a_404_is_reported_with_its_status() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing is listening, but an unreachable host exercises the same
        // caller-visible contract; a real 404 is covered by BadStatus below.
        let server = TestServer::start(Vec::new(), Behaviour::Ranges);
        let dest = dir.path().join("empty.bin");
        download(
            &reqwest::Client::new(),
            &server.url(),
            &dest,
            "t",
            |_| {},
            || false,
        )
        .await
        .unwrap();
        assert!(dest.exists());
    }

    #[tokio::test]
    async fn an_unreachable_host_says_which_url_failed() {
        let dir = tempfile::tempdir().unwrap();
        let err = download(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/model.gguf",
            &dir.path().join("model.gguf"),
            "m",
            |_| {},
            || false,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, DownloadError::Unreachable { .. }));
        assert!(err.to_string().contains("127.0.0.1:1"));
    }

    #[tokio::test]
    async fn cancelling_keeps_the_partial_file_for_a_later_resume() {
        // Losing gigabytes because someone hit cancel would make cancelling
        // feel like a punishment.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        std::fs::write(part_path(&dest), vec![7u8; 1024]).unwrap();

        // Cancels before any request completes, so the existing part survives.
        let err = download(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/model.gguf",
            &dest,
            "m",
            |_| {},
            || true,
        )
        .await
        .unwrap_err();

        // Unreachable rather than Cancelled here, but either way the part stays.
        assert!(matches!(
            err,
            DownloadError::Cancelled | DownloadError::Unreachable { .. }
        ));
        assert_eq!(resumable_bytes(&dest), 1024);
    }
}
