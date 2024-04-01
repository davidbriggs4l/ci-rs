use hyper::Uri as HyperUri;
use url::Url;

use std::borrow::Cow;
use std::ffi::OsStr;

use crate::docker::errors::Error;
use crate::docker::{ClientType, ClientVersion};

#[derive(Debug)]
pub struct Uri<'a> {
    encoded: Cow<'a, str>,
}
impl<'a> TryFrom<Uri<'a>> for HyperUri {
    type Error = http::uri::InvalidUri;

    fn try_from(uri: Uri<'a>) -> Result<Self, Self::Error> {
        uri.encoded.as_ref().parse()
    }
}

impl<'a> Uri<'a> {
    pub(crate) fn parse<T>(
        socket: &'a str,
        client_type: &ClientType,
        path: &'a str,
        query: Option<T>,
        client_version: &ClientVersion,
    ) -> Result<Self, Error>
    where
        T: serde::ser::Serialize,
    {
        //unix://
        let host_str = format!(
            "{}://{}/v{}.{}{}",
            Uri::socket_scheme(client_type),
            Uri::socket_host(socket, client_type),
            client_version.major_version,
            client_version.minor_version,
            path
        );
        let mut url = Url::parse(host_str.as_ref())?;
        url = url.join(path)?;

        if let Some(pairs) = query {
            let qs = serde_urlencoded::to_string(pairs)?;
            url.set_query(Some(&qs));
        }

        Ok(Uri {
            encoded: Cow::Owned(url.as_str().to_owned()),
        })
    }

    fn socket_host<P>(socket: P, client_type: &ClientType) -> String
    where
        P: AsRef<OsStr>,
    {
        match client_type {
            ClientType::Unix => hex::encode(socket.as_ref().to_string_lossy().as_bytes()),
        }
    }

    fn socket_scheme(client_type: &ClientType) -> &'a str {
        match client_type {
            ClientType::Unix => "unix",
        }
    }
}
