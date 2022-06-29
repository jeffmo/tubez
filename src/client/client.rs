use std::collections::HashMap;

use super::channel::Channel;
use super::channel::ChannelConnectError;

pub struct Client {
  hyper_client: hyper::Client<hyper::client::HttpConnector>,
}
impl Client {
  pub fn new() -> Self {
    let hyper_client: hyper::Client<hyper::client::HttpConnector> = 
      hyper::Client::builder()
        .http2_only(true)
        .build_http();

    Client {
      hyper_client,
    }
  }

  pub async fn make_tube_channel(
    &mut self,
    headers: HashMap<String, String>,
  ) -> Result<Channel, ChannelConnectError> {
    Channel::new(&self.hyper_client, headers).await
  }
}
