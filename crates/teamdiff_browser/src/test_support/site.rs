use std::{future, io};

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch::error::RecvError;
use tokio::sync::watch::{self, Receiver, Sender};

/// A finished match as tracker.gg's `/matches/{id}` returns it, cut to what the match trim reads.
const MATCH: &str = r#"{"data":{"segments":[
        {"type":"player-summary","attributes":{"platformUserIdentifier":"A#1"},"metadata":{"teamId":"Red","partyId":"1","agentName":"Jett"}},
        {"type":"player-summary","attributes":{"platformUserIdentifier":"B#2"},"metadata":{"teamId":"Red","partyId":"1"}},
        {"type":"player-summary","attributes":{"platformUserIdentifier":"C#3"},"metadata":{"teamId":"Blue","partyId":"1"}},
        {"type":"round-summary","attributes":{},"metadata":{}}
    ]}}"#;

/// A local site on 127.0.0.1 whose API is called from `localhost`, a different origin, the way
/// tracker.gg's page calls api.tracker.gg: CORS applies and `Retry-After` isn't exposed to the
/// page.
///
/// Both origins serve the same paths:
///
/// | Path | Answer |
/// | --- | --- |
/// | `/` | 200, an HTML page to sit on |
/// | `/blocked` | 403, an HTML block page without CORS headers |
/// | `/data` | 200, `{"ok":1}` |
/// | `/slow` | never answers the first request; 200, `{"ok":1}` to every later one |
/// | `/busy`, and any path that starts with it | 503 with `Retry-After: 7` |
/// | `/match` | 200, a finished match as tracker.gg returns it |
/// | `/empty` | 200, `{}` |
/// | `/html` | 200, an HTML body |
/// | anything else | 404 |
#[derive(Debug, Clone)]
pub struct Site {
    /// The page's origin, such as `http://127.0.0.1:50000`.
    pub page: String,
    /// The API's origin, such as `http://localhost:50000`.
    pub api: String,
    /// How many `/slow` requests have arrived.
    slow: Receiver<usize>,
}

impl Site {
    /// Starts the site on the current Tokio runtime, where it serves until the runtime stops.
    ///
    /// # Errors
    ///
    /// Fails when no local port can be bound.
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let page = format!("http://127.0.0.1:{port}");
        let cors = format!(
            "Access-Control-Allow-Origin: {page}\r\nAccess-Control-Allow-Credentials: true\r\n"
        );
        let (arrived, slow) = watch::channel(0);
        tokio::spawn(serve(listener, cors, arrived));
        Ok(Self {
            page,
            api: format!("http://localhost:{port}"),
            slow,
        })
    }

    /// Waits until at least `count` `/slow` requests have arrived, so a test can act while the
    /// first is still unanswered.
    ///
    /// # Errors
    ///
    /// Fails when the site stopped serving first.
    pub async fn slow_requests(&self, count: usize) -> Result<(), RecvError> {
        let mut slow = self.slow.clone();
        slow.wait_for(|arrived| *arrived >= count).await?;
        Ok(())
    }
}

/// Accepts connections until accepting fails, answering each on a task of its own.
async fn serve(listener: TcpListener, cors: String, slow: Sender<usize>) {
    while let Ok((socket, _)) = listener.accept().await {
        tokio::spawn(respond(socket, cors.clone(), slow.clone()));
    }
}

/// Reads a request's head and answers it by its path, as the table on [`Site`] says. `slow`
/// counts the `/slow` requests that arrive.
async fn respond(mut socket: TcpStream, cors: String, slow: Sender<usize>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.ends_with(b"\r\n\r\n") {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let path = String::from_utf8_lossy(&request)
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_owned();
    if path == "/slow" {
        let mut first = false;
        slow.send_modify(|arrived| {
            first = *arrived == 0;
            *arrived += 1;
        });
        if first {
            future::pending::<()>().await;
        }
    }
    let (status, extra, body) = match path.as_str() {
        "/" => (
            "200 OK",
            "Content-Type: text/html\r\n".to_owned(),
            "<p>home</p>",
        ),
        "/data" | "/slow" => (
            "200 OK",
            format!("Content-Type: application/json\r\n{cors}"),
            r#"{"ok":1}"#,
        ),
        p if p.starts_with("/busy") => (
            "503 Service Unavailable",
            format!("Retry-After: 7\r\n{cors}"),
            "",
        ),
        "/blocked" => (
            "403 Forbidden",
            "Content-Type: text/html\r\n".to_owned(),
            "<p>blocked</p>",
        ),
        "/match" => (
            "200 OK",
            format!("Content-Type: application/json\r\n{cors}"),
            MATCH,
        ),
        "/empty" => (
            "200 OK",
            format!("Content-Type: application/json\r\n{cors}"),
            "{}",
        ),
        "/html" => (
            "200 OK",
            format!("Content-Type: text/html\r\n{cors}"),
            "<p>not json</p>",
        ),
        _ => ("404 Not Found", cors, ""),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    #[expect(
        clippy::let_underscore_must_use,
        let_underscore_drop,
        reason = "a browser that hangs up before the answer is written needs nothing more from \
                  this connection"
    )]
    let _ = socket.write_all(response.as_bytes()).await;
}
