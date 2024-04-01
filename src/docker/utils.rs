use std::fmt;

use hyper::body::Bytes;
use serde::Serialize;

/// Result type for the [Logs API](Docker::logs())
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum LogOutput {
    StdErr { message: Bytes },
    StdOut { message: Bytes },
    StdIn { message: Bytes },
    Console { message: Bytes },
}

impl fmt::Display for LogOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match &self {
            LogOutput::StdErr { message } => message,
            LogOutput::StdOut { message } => message,
            LogOutput::StdIn { message } => message,
            LogOutput::Console { message } => message,
        };
        write!(f, "{}", String::from_utf8_lossy(message))
    }
}

impl AsRef<[u8]> for LogOutput {
    fn as_ref(&self) -> &[u8] {
        match self {
            LogOutput::StdErr { message } => message.as_ref(),
            LogOutput::StdOut { message } => message.as_ref(),
            LogOutput::StdIn { message } => message.as_ref(),
            LogOutput::Console { message } => message.as_ref(),
        }
    }
}

impl LogOutput {
    /// Get the raw bytes of the output
    pub fn into_bytes(self) -> Bytes {
        match self {
            LogOutput::StdErr { message } => message,
            LogOutput::StdOut { message } => message,
            LogOutput::StdIn { message } => message,
            LogOutput::Console { message } => message,
        }
    }
}

pub(crate) fn serialize_as_json<T, S>(t: &T, s: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: serde::Serializer,
{
    s.serialize_str(
        &serde_json::to_string(t).map_err(|e| serde::ser::Error::custom(format!("{e}")))?,
    )
}

pub(crate) fn serialize_join_newlines<S>(t: &[&str], s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&t.join("\n"))
}

#[cfg(feature = "time")]
pub fn deserialize_rfc3339<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<time::OffsetDateTime, D::Error> {
    let s: String = serde::Deserialize::deserialize(d)?;
    time::OffsetDateTime::parse(&s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| serde::de::Error::custom(format!("{:?}", e)))
}

#[cfg(feature = "time")]
pub fn serialize_rfc3339<S: serde::Serializer>(
    date: &time::OffsetDateTime,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(
        &date
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| serde::ser::Error::custom(format!("{:?}", e)))?,
    )
}

#[cfg(feature = "time")]
pub(crate) fn serialize_as_timestamp<S>(
    opt: &Option<crate::models::BollardDate>,
    s: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(t) => s.serialize_str(&format!(
            "{}.{}",
            t.unix_timestamp(),
            t.unix_timestamp_nanos()
        )),
        None => s.serialize_str(""),
    }
}

#[cfg(all(feature = "chrono", not(feature = "time")))]
pub(crate) fn serialize_as_timestamp<S>(
    opt: &Option<crate::models::BollardDate>,
    s: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(t) => s.serialize_str(&format!("{}.{}", t.timestamp(), t.timestamp_subsec_nanos())),
        None => s.serialize_str(""),
    }
}
