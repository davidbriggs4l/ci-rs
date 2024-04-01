use std::cmp;
use std::env;
use std::fmt;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use self::errors::Error;
use self::read::JsonLineDecoder;
use self::read::NewlineLogOutputDecoder;
use self::read::StreamReader;
use self::uri::Uri;
use self::utils::LogOutput;

use futures_core::Future;
use futures_core::Stream;
use futures_util::FutureExt;
use futures_util::TryFutureExt;
use futures_util::TryStreamExt;
use http::header::CONTENT_TYPE;
use http::request::Builder;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{self, body::Bytes, Method, Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyperlocal_next::UnixConnector;
use serde::de::DeserializeOwned;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use tokio_util::codec::FramedRead;

pub mod container;
pub mod errors;
pub mod read;
pub mod uri;
pub mod utils;

pub const DEFAULT_SOCKET: &str = "unix:///var/run/docker.sock";

pub const DEFAULT_DOCKER_HOST: &str = DEFAULT_SOCKET;

/// Default Client Version to communicate with the server.
pub const API_DEFAULT_VERSION: &ClientVersion = &ClientVersion {
    major_version: 1,
    minor_version: 42,
};

/// Default timeout for all requests is 2 minutes.
const DEFAULT_TIMEOUT: u64 = 120;

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct ClientVersion {
    pub minor_version: usize,
    pub major_version: usize,
}

impl fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major_version, self.minor_version)
    }
}

impl PartialOrd for ClientVersion {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        match self.major_version.partial_cmp(&other.major_version) {
            Some(cmp::Ordering::Equal) => self.minor_version.partial_cmp(&other.minor_version),
            res => res,
        }
    }
}

impl From<&(AtomicUsize, AtomicUsize)> for ClientVersion {
    fn from(tpl: &(AtomicUsize, AtomicUsize)) -> ClientVersion {
        ClientVersion {
            major_version: tpl.0.load(Ordering::Relaxed),
            minor_version: tpl.1.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ClientType {
    Unix,
}

pub(crate) enum Transport {
    Unix {
        client: Client<UnixConnector, Full<Bytes>>,
    },
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Transport::Unix { .. } => write!(f, "Unix"),
        }
    }
}

/// Internal model: Docker Server JSON payload when an error is emitted
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct DockerServerErrorMessage {
    message: String,
}

#[derive(Debug)]
pub struct Docker {
    pub(crate) transport: Arc<Transport>,
    pub(super) client_type: ClientType,
    pub(crate) client_addr: String,
    pub(crate) client_timeout: u64,
    pub(crate) version: Arc<(AtomicUsize, AtomicUsize)>,
}
impl Clone for Docker {
    fn clone(&self) -> Docker {
        Docker {
            transport: self.transport.clone(),
            client_type: self.client_type.clone(),
            client_addr: self.client_addr.clone(),
            client_timeout: self.client_timeout,
            version: self.version.clone(),
        }
    }
}

impl Docker {
    pub fn connect_with_unix_defaults() -> Result<Docker, Error> {
        let socket_path = env::var("DOCKER_HOST").ok().and_then(|p| {
            if p.starts_with("unix://") {
                Some(p)
            } else {
                None
            }
        });
        let path = socket_path.as_deref();
        let path_ref = path.unwrap_or(DEFAULT_SOCKET);
        Docker::connect_with_unix(path_ref, DEFAULT_TIMEOUT, API_DEFAULT_VERSION)
    }
    pub fn connect_with_unix(
        path: &str,
        timeout: u64,
        client_version: &ClientVersion,
    ) -> Result<Docker, Error> {
        let client_addr = path.replacen("unix://", "", 1);
        let unix_connector = UnixConnector;
        let client_builder = Client::builder(TokioExecutor::new());

        let client = client_builder.build(unix_connector);
        let transport = Transport::Unix { client };
        Ok(Docker {
            client_addr,
            client_timeout: timeout,
            transport: Arc::new(transport),
            client_type: ClientType::Unix,
            version: Arc::new((
                AtomicUsize::new(client_version.major_version),
                AtomicUsize::new(client_version.minor_version),
            )),
        })
    }
}

impl Docker {
    /// By default, 2 minutes.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.set_timeout(timeout);
        self
    }

    /// Get the current timeout.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.client_timeout)
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.client_timeout = timeout.as_secs();
    }
}

impl Docker {
    pub(crate) fn process_into_stream<T>(
        &self,
        req: Result<Request<Full<Bytes>>, Error>,
    ) -> impl Stream<Item = Result<T, Error>> + Unpin
    where
        T: DeserializeOwned,
    {
        Box::pin(
            self.process_request(req)
                .map_ok(Docker::decode_into_stream::<T>)
                .into_stream()
                .try_flatten(),
        )
    }

    pub(crate) fn process_into_stream_string(
        &self,
        req: Result<Request<Full<Bytes>>, Error>,
    ) -> impl Stream<Item = Result<LogOutput, Error>> + Unpin {
        Box::pin(
            self.process_request(req)
                .map_ok(Docker::decode_into_stream_string)
                .try_flatten_stream(),
        )
    }
    pub(crate) fn process_into_unit(
        &self,
        req: Result<Request<Full<Bytes>>, Error>,
    ) -> impl Future<Output = Result<(), Error>> {
        let fut = self.process_request(req);
        async move {
            fut.await?;
            Ok(())
        }
    }
    pub(crate) fn process_into_value<T>(
        &self,
        req: Result<Request<Full<Bytes>>, Error>,
    ) -> impl Future<Output = Result<T, Error>>
    where
        T: DeserializeOwned,
    {
        let fut = self.process_request(req);
        async move { Docker::decode_response(fut.await?).await }
    }
    pub(crate) fn serialize_payload<S>(body: Option<S>) -> Result<Full<Bytes>, Error>
    where
        S: serde::Serialize,
    {
        match body.map(|inst| serde_json::to_string(&inst)) {
            Some(Ok(res)) => Ok(Some(res)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
        .map(|payload| {
            payload
                .map(|content| Full::new(content.into()))
                .unwrap_or(Full::new(Bytes::new()))
        })
    }

    /// Return the currently set client version.
    pub fn client_version(&self) -> ClientVersion {
        self.version.as_ref().into()
    }
    fn decode_into_stream<T>(res: Response<Incoming>) -> impl Stream<Item = Result<T, Error>>
    where
        T: DeserializeOwned,
    {
        FramedRead::new(StreamReader::new(res.into_body()), JsonLineDecoder::new())
    }

    fn decode_into_stream_string(
        res: Response<Incoming>,
    ) -> impl Stream<Item = Result<LogOutput, Error>> {
        FramedRead::new(
            StreamReader::new(res.into_body()),
            NewlineLogOutputDecoder::new(false),
        )
        .map_err(Error::from)
    }
    pub(crate) fn build_request<O>(
        &self,
        path: &str,
        builder: Builder,
        query: Option<O>,
        payload: Result<Full<Bytes>, Error>,
    ) -> Result<Request<Full<Bytes>>, Error>
    where
        O: serde::Serialize,
    {
        let uri = Uri::parse(
            &self.client_addr,
            &self.client_type,
            path,
            query,
            &self.client_version(),
        )?;
        let req_uri: hyper::Uri = uri.try_into()?;
        Ok(builder
            .uri(req_uri)
            .header(CONTENT_TYPE, "application/json")
            .body(payload?)?)
    }

    pub(crate) fn process_request(
        &self,
        request: Result<Request<Full<Bytes>>, Error>,
    ) -> impl Future<Output = Result<Response<Incoming>, Error>> {
        let transport = self.transport.clone();
        let timeout = self.client_timeout;

        async move {
            let request = request?;
            let response = Docker::execute_request(transport, request, timeout).await?;

            let status = response.status();
            match status {
                // Status code 200 - 299 or 304
                s if s.is_success() || s == StatusCode::NOT_MODIFIED => Ok(response),

                StatusCode::SWITCHING_PROTOCOLS => Ok(response),

                // All other status codes
                _ => {
                    let contents = Docker::decode_into_string(response).await?;

                    let mut message = String::new();
                    if !contents.is_empty() {
                        message = serde_json::from_str::<DockerServerErrorMessage>(&contents)
                            .map(|msg| msg.message)
                            .or_else(|e| if e.is_data() { Ok(contents) } else { Err(e) })?;
                    }
                    Err(Error::DockerResponseServerError {
                        status_code: status.as_u16(),
                        message,
                    })
                }
            }
        }
    }
    async fn execute_request(
        transport: Arc<Transport>,
        req: Request<Full<Bytes>>,
        timeout: u64,
    ) -> Result<Response<Incoming>, Error> {
        // This is where we determine to which transport we issue the request.
        let request = match *transport {
            Transport::Unix { ref client } => client.request(req),
        };

        match tokio::time::timeout(Duration::from_secs(timeout), request).await {
            Ok(v) => Ok(v?),
            Err(_) => Err(Error::RequestTimeoutError),
        }
    }
    async fn decode_into_string(response: Response<Incoming>) -> Result<String, Error> {
        let body = response.into_body().collect().await?.to_bytes();

        Ok(String::from_utf8_lossy(&body).to_string())
    }
    async fn decode_response<T>(response: Response<Incoming>) -> Result<T, Error>
    where
        T: DeserializeOwned,
    {
        let bytes = response.into_body().collect().await?.to_bytes();

        serde_json::from_slice::<T>(&bytes).map_err(|e| {
            if e.is_data() {
                Error::JsonDataError {
                    message: e.to_string(),
                    column: e.column(),
                    // #[cfg(feature = "json_data_content")]
                    contents: String::from_utf8_lossy(&bytes).to_string(),
                }
            } else {
                e.into()
            }
        })
    }
}
