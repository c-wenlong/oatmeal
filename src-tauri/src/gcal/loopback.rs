//! The one-shot HTTP server that catches Google's redirect.
//!
//! Google's recommended redirect for a desktop app is a loopback address —
//! `http://127.0.0.1:<port>` — because it needs no custom URL scheme and cannot
//! be claimed by another app the way a scheme can. The port is chosen by the OS
//! at flow time; Google does not check it for loopback redirects, so nothing
//! has to be registered in advance.
//!
//! Three properties this has to hold, all of them easy to get wrong:
//!
//! 1. **Loopback only.** Binding `0.0.0.0` would put an endpoint that accepts
//!    authorization codes on the local network.
//! 2. **One request.** The listener closes as soon as it has an answer, so
//!    nothing is left listening after the flow.
//! 3. **The code never reaches the page.** The browser tab is shown a plain
//!    "you can close this" — putting the code in the HTML would leave it in
//!    the page source, and in the browser's history alongside the URL.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::Duration;

use super::pkce::{self, Callback};

#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    #[error("could not listen on loopback: {0}")]
    Bind(String),
    #[error("the browser never came back")]
    TimedOut,
    #[error("the redirect carried nothing usable")]
    Unusable,
}

/// A listener waiting for exactly one redirect.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// Binds an ephemeral port on 127.0.0.1.
    pub fn bind() -> Result<Self, LoopbackError> {
        // Port 0 asks the OS for a free one. Loopback only — see the module
        // note; `0.0.0.0` here would be a network-visible token endpoint.
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| LoopbackError::Bind(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| LoopbackError::Bind(e.to_string()))?
            .port();
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The exact string that must also go in the authorization request.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Waits for the redirect, answers the browser, and returns what it carried.
    ///
    /// Consumes `self`: the listener is dropped on return, so nothing is left
    /// accepting connections once the flow is over.
    pub fn wait(self, timeout: Duration) -> Result<Callback, LoopbackError> {
        // Non-blocking, because a blocking `accept` only returns when something
        // connects — so the deadline below would only be consulted *between*
        // connections, and a user who closes the browser without answering
        // would leave this waiting forever on a bound port.
        self.listener
            .set_nonblocking(true)
            .map_err(|e| LoopbackError::Bind(e.to_string()))?;

        let deadline = std::time::Instant::now() + timeout;
        // A browser opens more than one connection to a host — favicon requests
        // and speculative preconnects both arrive here. Anything that is not the
        // redirect is answered and ignored rather than ending the flow.
        while std::time::Instant::now() < deadline {
            let (mut stream, _) = match self.listener.accept() {
                Ok(accepted) => accepted,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    // Nothing waiting. Sleep briefly rather than spinning a core
                    // for the several minutes a consent screen can take.
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => continue,
            };
            // The socket itself goes back to blocking: the request is read with
            // a timeout, and a non-blocking read would return WouldBlock before
            // the browser had finished sending its headers.
            let _ = stream.set_nonblocking(false);
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let _ = stream.set_read_timeout(Some(remaining.min(Duration::from_secs(10))));

            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }

            // "GET /?code=…&state=… HTTP/1.1"
            let Some(target) = line.split_whitespace().nth(1) else {
                continue;
            };
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");

            match pkce::parse_callback(query) {
                Some(callback) => {
                    let _ = stream.write_all(response_page(&callback).as_bytes());
                    let _ = stream.flush();
                    return Ok(callback);
                }
                None => {
                    // Not the redirect. Answer so the browser is not left
                    // hanging, and keep waiting for the real one.
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        }
        Err(LoopbackError::TimedOut)
    }
}

/// What the browser tab is shown.
///
/// Deliberately carries **no code and no state** — the page source and the
/// browser's history would both keep them. It says one thing and closes.
pub fn response_page(callback: &Callback) -> String {
    let body = match callback {
        Callback::Code { .. } => "<h1>Oatmeal is connected</h1><p>You can close this tab.</p>",
        Callback::Denied { .. } => {
            "<h1>Not connected</h1><p>Oatmeal was not given access. You can close this tab.</p>"
        }
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8>\
         <title>Oatmeal</title>\
         <style>body{{font:16px -apple-system,sans-serif;margin:12vh auto;max-width:28rem;\
         text-align:center;color:#3d3ا}}h1{{font-size:20px}}</style>{body}"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_binds_loopback_only() {
        // `0.0.0.0` would expose an endpoint that accepts authorization codes
        // to anything on the local network.
        let loopback = Loopback::bind().unwrap();
        let addr = loopback.listener.local_addr().unwrap();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn the_redirect_uri_matches_what_it_bound() {
        // A mismatch here is a `redirect_uri_mismatch` from Google that reads
        // like a console problem.
        let loopback = Loopback::bind().unwrap();
        assert_eq!(
            loopback.redirect_uri(),
            format!("http://127.0.0.1:{}", loopback.port())
        );
    }

    #[test]
    fn two_flows_get_different_ports() {
        let first = Loopback::bind().unwrap();
        let second = Loopback::bind().unwrap();
        assert_ne!(first.port(), second.port());
    }

    #[test]
    fn the_success_page_never_contains_the_code() {
        // It would end up in the page source and in browser history.
        let page = response_page(&Callback::Code {
            code: "4/0SUPERSECRET".into(),
            state: "st4te".into(),
        });
        assert!(
            !page.contains("4/0SUPERSECRET"),
            "the code leaked into the page"
        );
        assert!(!page.contains("st4te"));
        assert!(page.contains("close this tab"));
    }

    #[test]
    fn the_denial_page_says_what_happened() {
        let page = response_page(&Callback::Denied {
            error: "access_denied".into(),
            state: None,
        });
        assert!(page.contains("Not connected"));
        // The raw error is for the log, not the browser.
        assert!(!page.contains("access_denied"));
    }

    #[test]
    fn the_response_declares_its_length() {
        // Without Content-Length the browser waits for the socket to close and
        // the tab appears to hang after a successful connection.
        let page = response_page(&Callback::Code {
            code: "c".into(),
            state: "s".into(),
        });
        let body = page.split_once("\r\n\r\n").unwrap().1;
        assert!(page.contains(&format!("Content-Length: {}", body.len())));
    }

    #[test]
    fn it_gives_up_rather_than_waiting_forever() {
        // A blocking `accept` only returns when something connects, so the
        // deadline would only be checked between connections — and a user who
        // closes the browser would leave this waiting on a bound port forever.
        let loopback = Loopback::bind().unwrap();
        let started = std::time::Instant::now();

        let result = loopback.wait(Duration::from_millis(200));

        assert!(matches!(result, Err(LoopbackError::TimedOut)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout was not honoured: waited {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_port_is_released_once_the_flow_ends() {
        // `wait` consumes the listener, so nothing is left accepting
        // authorization codes after the flow is over.
        let loopback = Loopback::bind().unwrap();
        let port = loopback.port();
        let _ = loopback.wait(Duration::from_millis(100));

        // Re-binding the same port proves the old listener is gone.
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "the loopback port was still held after the flow ended"
        );
    }

    #[test]
    fn it_receives_a_real_redirect() {
        use std::io::Read;
        let loopback = Loopback::bind().unwrap();
        let port = loopback.port();

        std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(b"GET /?code=abc&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
        });

        let callback = loopback.wait(Duration::from_secs(5)).unwrap();
        assert_eq!(
            callback,
            Callback::Code {
                code: "abc".into(),
                state: "xyz".into()
            }
        );
    }

    #[test]
    fn a_stray_request_does_not_end_the_flow() {
        // Browsers ask for /favicon.ico and speculatively preconnect. Treating
        // the first connection as the answer would abandon the real redirect.
        use std::io::Read;
        let loopback = Loopback::bind().unwrap();
        let port = loopback.port();

        std::thread::spawn(move || {
            let mut noise = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let _ = noise.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n");
            let mut sink = String::new();
            let _ = noise.read_to_string(&mut sink);

            let mut real = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let _ = real.write_all(b"GET /?code=real&state=xyz HTTP/1.1\r\nHost: x\r\n\r\n");
            let mut response = String::new();
            let _ = real.read_to_string(&mut response);
        });

        let callback = loopback.wait(Duration::from_secs(5)).unwrap();
        assert_eq!(
            callback,
            Callback::Code {
                code: "real".into(),
                state: "xyz".into()
            }
        );
    }
}
