use std::fmt::format;
use std::{collections::HashMap, hash::Hash};

use derive_new::new;
use futures_core::Stream;
use futures_util::StreamExt;
use http::request::Builder;
use http::Method;
use http_body_util::Full;
use hyper::body::Bytes;
use serde_derive::{Deserialize, Serialize};

use bollard_stubs::models::*;

use super::errors::Error;
use super::Docker;

#[derive(Debug, Clone, Default, PartialEq, Serialize, new)]
pub struct CreateContainerOptions<T>
where
    T: Into<String> + serde::Serialize,
{
    pub name: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<T>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, new)]
pub struct CreateContainerConfig<T> {
    #[serde(rename = "Image")]
    pub image: T,
    #[serde(rename = "Tty")]
    pub tty: bool,
    #[serde(rename = "Labels")]
    pub labels: HashMap<String, T>,
    #[serde(rename = "Entrypoint")]
    pub entry_point: Vec<T>,
    #[serde(rename = "Cmd")]
    pub cmd: T,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, new)]
#[serde(rename_all = "camelCase")]
pub struct StartContainerOptions<T>
where
    T: Into<String> + serde::Serialize,
{
    pub detach_keys: T,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, new)]
pub struct WaitContainerOptions<T>
where
    T: Into<String> + serde::Serialize,
{
    /// Wait until a container state reaches the given condition, either 'not-running' (default),
    /// 'next-exit', or 'removed'.
    pub condition: T,
}

impl Docker {
    pub async fn create_container<T, Z>(
        &self,
        options: Option<CreateContainerOptions<T>>,
        config: CreateContainerConfig<Z>,
    ) -> Result<ContainerCreateResponse, Error>
    where
        T: Into<String> + serde::Serialize,
        Z: Into<String> + Hash + Eq + serde::Serialize,
    {
        let url = "/containers/create";
        let req = self.build_request(
            url,
            Builder::new().method(Method::POST),
            options,
            Docker::serialize_payload(Some(config)),
        );
        self.process_into_value(req).await
    }

    pub async fn start_container<T>(
        &self,
        container_name_or_id: &str,
        query: Option<StartContainerOptions<T>>,
    ) -> Result<(), Error>
    where
        T: Into<String> + serde::Serialize,
    {
        let path = format!("/containers/{container_name_or_id}/start");
        let req = self.build_request(
            &path,
            Builder::new().method(Method::POST),
            query,
            Ok(Full::new(Bytes::new())),
        );
        self.process_into_unit(req).await
    }

    pub fn wait_container<T>(
        &self,
        container_name_or_id: &str,
        options: Option<WaitContainerOptions<T>>,
    ) -> impl Stream<Item = Result<ContainerWaitResponse, Error>>
    where
        T: Into<String> + serde::Serialize,
    {
        let path = format!("/containers/{container_name_or_id}/wait");
        let req = self.build_request(
            &path,
            Builder::new().method(Method::POST),
            options,
            Ok(Full::new(Bytes::new())),
        );

        self.process_into_stream(req).map(|res| match res {
            Ok(ContainerWaitResponse {
                status_code: code,
                error:
                    Some(ContainerWaitResponseError {
                        message: Some(error),
                    }),
            }) if code > 0 => Err(Error::DockerContainerWaitError { error, code }),
            Ok(ContainerWaitResponse {
                status_code: code,
                error: None,
            }) if code > 0 => Err(Error::DockerContainerWaitError {
                error: String::new(),
                code,
            }),
            v => v,
        })
    }
}
