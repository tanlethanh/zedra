// CfWorkerDiscovery: implements iroh's AddressLookup trait backed by the
// Cloudflare Worker coordination server. Hosts publish their EndpointAddr,
// clients resolve by EndpointId.

use std::collections::BTreeSet;

use futures::stream::BoxStream;
use futures::StreamExt;
use iroh::address_lookup::{self, AddressLookup, EndpointData, EndpointInfo, Item};
use iroh::{EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};

/// Discovery service backed by the Zedra Cloudflare Worker.
///
/// The CF Worker stores a mapping from endpoint ID (z-base-32 encoded) to
/// the endpoint's addressing data (relay URL + direct addresses).
#[derive(Debug, Clone)]
pub struct CfWorkerDiscovery {
    coord_url: String,
    client: reqwest::Client,
}

/// Payload sent to `POST /publish`.
#[derive(Serialize)]
struct PublishRequest {
    endpoint_id: String,
    relay_url: Option<String>,
    direct_addrs: Vec<String>,
}

/// Response from `GET /resolve/:endpoint_id`.
#[derive(Deserialize)]
struct ResolveResponse {
    #[allow(dead_code)]
    endpoint_id: String,
    relay_url: Option<String>,
    direct_addrs: Vec<String>,
}

impl CfWorkerDiscovery {
    pub fn new(coord_url: &str) -> Self {
        Self {
            coord_url: coord_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }
}

impl AddressLookup for CfWorkerDiscovery {
    fn publish(&self, _data: &EndpointData) {
        // Fire-and-forget: the host publishes via the registration loop
        // since publish() doesn't have access to the endpoint ID.
        // See publish_endpoint() below.
    }

    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<BoxStream<'static, Result<Item, address_lookup::Error>>> {
        let url = format!("{}/resolve/{}", self.coord_url, endpoint_id);
        let client = self.client.clone();

        Some(
            futures::stream::once(async move {
                let resp = client
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| address_lookup::Error::from_err("cf-worker", e))?;

                if !resp.status().is_success() {
                    return Err(address_lookup::Error::from_err(
                        "cf-worker",
                        std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("resolve returned status {}", resp.status()),
                        ),
                    ));
                }

                let body: ResolveResponse = resp
                    .json()
                    .await
                    .map_err(|e| address_lookup::Error::from_err("cf-worker", e))?;

                // Build EndpointData from the response
                let mut endpoint_data = EndpointData::new(std::iter::empty());

                if let Some(ref relay) = body.relay_url {
                    if let Ok(relay_url) = relay.parse::<RelayUrl>() {
                        endpoint_data = endpoint_data.with_relay_url(Some(relay_url));
                    }
                }

                let addrs: BTreeSet<std::net::SocketAddr> = body
                    .direct_addrs
                    .iter()
                    .filter_map(|a| a.parse().ok())
                    .collect();
                if !addrs.is_empty() {
                    endpoint_data = endpoint_data.with_ip_addrs(addrs);
                }

                let info = EndpointInfo::from_parts(endpoint_id, endpoint_data);
                Ok(Item::new(info, "cf-worker", None))
            })
            .boxed(),
        )
    }
}

/// Publish the endpoint's addressing info to the CF Worker.
///
/// Called from the host's registration loop since we need the endpoint ID
/// which isn't available inside the `AddressLookup::publish()` callback.
pub async fn publish_endpoint(
    coord_url: &str,
    endpoint_id: &EndpointId,
    relay_url: Option<&RelayUrl>,
    direct_addrs: &[std::net::SocketAddr],
) -> anyhow::Result<()> {
    let url = format!(
        "{}/publish",
        coord_url.trim_end_matches('/')
    );

    let body = PublishRequest {
        endpoint_id: endpoint_id.to_string(),
        relay_url: relay_url.map(|u| u.to_string()),
        direct_addrs: direct_addrs.iter().map(|a| a.to_string()).collect(),
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("publish failed with status {}", resp.status());
    }

    Ok(())
}
