use std::{str::FromStr, sync::Arc, time::{Instant, SystemTime}};

use tokio::{io::{self, ErrorKind, AsyncReadExt, AsyncWriteExt}, net::TcpStream};
use tokio_rustls::{rustls, client::TlsStream};

use super::Body;

pub type Stream = TlsStream<TcpStream>;

pub struct PingResult {
    pub server_time: SystemTime,
    pub machine_time: SystemTime,
    pub initiated_at: Instant
}

pub struct Client {
    host: String,
    stream: Stream
}

impl Client {
    pub async fn new(host: impl ToString, port: u16) -> io::Result<Self> {
        let host = host.to_string();
        let stream = Self::_connect_tls(&host, port).await?;
        Ok( Self { host, stream } )
    }

    async fn _connect_tls(host: &str, port: u16) -> io::Result<TlsStream<TcpStream>> {
        use tokio_rustls::TlsConnector;
        use rustls::pki_types::ServerName;

        let stream = TcpStream::connect((host, port)).await?;

        let domain = ServerName::try_from(host)
            .map_err(|_| ErrorKind::AddrNotAvailable)?
            .to_owned();
    
        TlsConnector::from(_config())
            .connect(domain, stream)
            .await
    }

    fn _prepare_body<W: std::io::Write> (&self, body: &mut W) -> io::Result<()> {
        write!(body, "GET / HTTP/1.1\r\n")?;
        write!(body, "Host: {}\r\n", self.host)?;
        write!(body, "Connection: keep-alive\r\n")?;
        write!(body, "Content-Length: 0\r\n")?;
        write!(body, "\r\n")?;
        
        Ok(())
    }

    async fn _parse_http<'a> (&'a mut self, buf: &'a mut Vec<u8>) -> io::Result<http::Response<Body<'a, Stream>>> {
        use http::response::Parts;
        use httparse::*;

        fn to_http_parts(response: Response) -> io::Result<Parts> {
            let (mut parts, _) = http::Response::new(()).into_parts();

            parts.status = match response.code.map(TryInto::try_into) {
                Some(Ok(x)) => x,
                Some(Err(_)) => Err(ErrorKind::InvalidData)?,
                _ => unreachable!()
            };

            parts.version = http::Version::HTTP_11;
            parts.headers = http::HeaderMap::try_with_capacity(response.headers.len())
                .or(Err(ErrorKind::OutOfMemory))?;

            for &mut Header { name, value } in response.headers {
                let name = http::HeaderName::from_str(name)
                    .or(Err(ErrorKind::InvalidData))?;

                let value = http::HeaderValue::from_bytes(value)
                    .or(Err(ErrorKind::InvalidData))?;

                parts.headers.append(name, value);
            }
            parts.extensions = http::Extensions::new();
            
            Ok(parts)
        }

        while self.stream.read_buf(buf).await? != 0 {
            let mut headers = [EMPTY_HEADER; 16];
            let mut response = Response::new(&mut headers);

            let pos = match response.parse(&buf[..]) {
                Ok(Status::Complete(x)) => x,
                Ok(Status::Partial) => continue,
                Err(e) => Err(io::Error::new(ErrorKind::InvalidData, e))?
            };

            let parts = to_http_parts(response)?;
            let body = self.to_http_body(&parts, buf, pos);

            return Ok(http::Response::from_parts(parts, body));
        }

        Err(ErrorKind::UnexpectedEof)?
    }

    fn to_http_body<'a> (&'a mut self, parts: &http::response::Parts, buf: &'a mut Vec<u8>, pos: usize) -> Body<'a, Stream> {
        use std::pin::Pin;
        use http::header::{HeaderValue, CONTENT_LENGTH};

        fn to_int(value: &HeaderValue) -> Option<usize> {
            let mut int = 0;
            for &byte in value.as_bytes() {
                let x = byte.checked_sub(48)?;
                assert!(x < 10);
                int = int * 10 + x as usize;
            }
            Some( int )
        }

        let len = parts.headers.get(CONTENT_LENGTH)
            .and_then(to_int);
        
        let stream = Pin::new(&mut self.stream);

        Body::new(stream, buf, pos, len)
    }

    /// Returns a tuple of (<DATE returned from server>, <TIME of response generation>).
    ///
    /// The precision of the DATE is 1 second; therefore, the DATE of the server
    /// at the time of request generation may be [<DATE>, <DATE> + 1s)
    /// 
    /// The TIME is assumed to be at the middlepoint of send and recv actions.
    pub async fn get_date(&mut self) -> io::Result<PingResult> {
        let mut buf = Vec::with_capacity(1024);
        
        self._prepare_body(&mut buf)?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        buf.clear();
        
        let now = Instant::now();
        
        let ping = {
            let response = self._parse_http(&mut buf).await?;

            let date = {
                let value = response.headers().get(http::header::DATE)
                    .ok_or(ErrorKind::NotFound)?;

                std::str::from_utf8(value.as_bytes())
                    .or(Err(ErrorKind::InvalidData))?
            };

            let server_time = httpdate::parse_http_date(&date)?;
            let machine_time = SystemTime::now().checked_sub(now.elapsed() / 2).unwrap();

            response.into_body().discard().await?;

            PingResult {
                server_time, machine_time, 
                initiated_at: now
            }
        };

        Ok(ping)
    }
}

fn _config() -> Arc<rustls::ClientConfig> {
    use std::sync::OnceLock;
    use rustls::ClientConfig;

    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

    fn _create_config() -> Arc<ClientConfig> {
        use rustls::RootCertStore;

        let root_store = RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS
                .iter()
                .cloned()
        );

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Arc::new(config)
    }
    
    CONFIG.get_or_init(_create_config).clone()
}