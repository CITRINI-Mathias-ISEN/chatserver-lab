use std::collections::HashMap;

use async_trait::async_trait;

use crate::messages::{
  ClientError, ClientId, ClientMessage, ClientPollReply, ClientReply, Sequence, ServerId,
};
#[cfg(feature = "federation")]
use crate::messages::{ServerMessage, ServerReply};

pub const MAILBOX_SIZE: usize = 256;
pub const WORKPROOF_STRENGTH: u32 = 8;

#[async_trait]
pub trait MessageServer {
  /// group name
  const GROUP_NAME: &'static str;

  /// create a new server, this is the constructor function
  fn new(id: ServerId) -> Self;

  /// register a new client, that will then be able to send and receive messages.
  /// The first argument is the client screen name.
  async fn register_local_client(&self, name: String) -> ClientId;

  /// list known users
  /// also lists known remote users if federation is enabled
  async fn list_users(&self) -> HashMap<ClientId, String>;

  /// handles a sequenced message
  /// you must verify:
  ///  * the workproof first, and then,
  ///  * that sequence numbers are increasing
  async fn handle_sequenced_message<A: Send>(&self, msg: Sequence<A>) -> Result<A, ClientError>;

  /// pull function for the client
  async fn client_poll(&self, client: ClientId) -> ClientPollReply;

  /// handles a client message
  /// * if the user is unknown, it might be that it is remote, so messages should be kept until the user becomes known
  ///   as a result, the "Delayed" message should be sent
  /// * until polled, messages are to be stored. There is a maximum mailbox size after which an error should be returned
  async fn handle_client_message(&self, src: ClientId, msg: ClientMessage) -> Vec<ClientReply>;

  #[cfg(feature = "federation")]
  /// handles a server message
  /// * might be an announce (which might trigger waiting messages to be sent)
  /// * might be a message for this server, or another
  async fn handle_server_message(&self, msg: ServerMessage) -> ServerReply;

  #[cfg(feature = "federation")]
  /// gives the best route to a server
  /// as a first approximation, you can give any route
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>>;
}
