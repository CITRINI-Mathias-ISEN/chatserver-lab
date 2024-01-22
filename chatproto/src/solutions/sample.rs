use async_std::sync::RwLock;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

use crate::{
  core::{MessageServer, MAILBOX_SIZE, WORKPROOF_STRENGTH},
  messages::{
    ClientError, ClientId, ClientMessage, ClientPollReply, ClientReply, FullyQualifiedMessage,
    Sequence, ServerId,
  },
  workproof::verify_workproof,
};

#[cfg(feature = "federation")]
use crate::messages::{Outgoing, ServerMessage, ServerReply};
use crate::messages::ClientQuery;
use crate::netproto::decode::u128;

// this structure will contain the data you need to track in your server
// this will include things like delivered messages, clients last seen sequence number, etc.
pub struct Server {
  id : ServerId,
  clients : RwLock<Vec<(ClientId, String, u64, Vec<(ClientId, String)>)>>,
  routes : RwLock<Vec<Vec<ServerId>>>,
  stored : RwLock<Vec<(ServerId, ClientId, String)>>,
}

pub struct Client {
  id : ClientId,
  name : String,
  last_seen : u64,
  mailbox : Vec<(ClientId, String)>,
}

#[async_trait]
impl MessageServer for Server {
  const GROUP_NAME: &'static str = "CITRINI Mathias, REPPLINGER Julien, VASSOILLE Rémi";

  fn new(id: ServerId) -> Self {
    Self { id, clients: Default::default(), routes: Default::default(), stored: Default::default() }
  }

  // note: you need to roll a Uuid, and then convert it into a ClientId
  // Uuid::new_v4() will generate such a value
  // you will most likely have to edit the Server struct as as to store information about the client
  async fn register_local_client(&self, name: String) -> ClientId {
    let uuid = Uuid::new_v4();
    let client_id = ClientId::from(uuid);
    let mut clients = self.clients.write().await;
    log::info!("Registering client {} with id {}", name, client_id);
    clients.push((client_id, name, 0, Vec::new()));
    return client_id;
  }

  /*
   implementation notes:
   * the workproof should be checked first
    * the nonce is in sequence.src and should be converted with (&sequence.src).into()
   * then, if the client is known, its last seen sequence number must be verified (and updated)
  */
  async fn list_users(&self) -> HashMap<ClientId, String> {
    let clients = self.clients.read().await;
    let mut users = HashMap::new();
    for (id, name, _, _) in clients.iter() {
      users.insert(*id, name.clone());
    }
    users
  }

  /* Here client messages are handled.
    * if the client is local,
      * if the mailbox is full, BoxFull should be returned
      * otherwise, Delivered should be returned
    * if the client is unknown, the message should be stored and Delayed must be returned
    * (federation) if the client is remote, Transfer should be returned

    It is recommended to write an function that handles a single message and use it to handle
    both ClientMessage variants.
  */
  async fn handle_sequenced_message<A: Send>(&self, sequence: Sequence<A>) -> Result<A, ClientError> {
    log::debug!("Handling sequenced message from {} with seqid {}", sequence.src, sequence.seqid);

    // check workproof
    if !verify_workproof((&sequence.src).into(), sequence.workproof, WORKPROOF_STRENGTH) {
      log::debug!("Workproof error");
      return Err(ClientError::WorkProofError);
    }

    // check if client is known
    let clients = self.clients.read().await;
    let client = clients.iter().find(|(id, _, _, _)| *id == sequence.src);
    if let Some((_, name, last_seen, _)) = client {
      log::debug!("Message from known client : {}", name);

      // check if sequence number is correct
      log::debug!("received: {}", sequence.seqid);
      if *last_seen >= (sequence.seqid as u64) {
        log::debug!("Sequence error current: {}, received: {}", last_seen, sequence.seqid);
        return Err(ClientError::SequenceError);
      }

      // Closing the read lock to avoid deadlock
      drop(clients);

      // update last seen sequence number
      let mut clients = self.clients.write().await;
      if let Some((_, _, last_seen, _)) = clients.iter_mut().find(|(id, _, _, _)| *id == sequence.src) {
      *last_seen = sequence.seqid as u64;
      } else {
        log::debug!("Unknown client");
        return Err(ClientError::UnknownClient);
      }
    } else {
      log::debug!("Unknown client");
      return Err(ClientError::UnknownClient);
    }
    log::debug!("Handling sequenced message done!");
    return Ok(sequence.content);
  }

  /* for the given client, return the next message or error if available */
  async fn client_poll(&self, client: ClientId) -> ClientPollReply {
    log::debug!("Polling client {:?}", client);

    // Opening the write lock to remove the message from the mailbox
    let mut clients = self.clients.write().await;
    let client_elem = clients.iter_mut().find(|(id, _, _, _)| *id == client);

    if let Some((_, _, _, mailbox)) = client_elem {
      if mailbox.is_empty() {
        return ClientPollReply::Nothing;
      }

      // Get the first message
      let (src, msg) = mailbox.remove(0);

      ClientPollReply::Message { src, content: msg }
    } else {
      return ClientPollReply::Nothing;
    }
  }

  /* For announces
     * if the route is empty, return EmptyRoute
     * if not, store the route in some way
     * also store the remote clients
     * if one of these remote clients has messages waiting, return them
    For messages
     * if local, deliver them
     * if remote, forward them
  */
  async fn handle_client_message(&self, src: ClientId, msg: ClientMessage) -> Vec<ClientReply> {
    match msg {
      ClientMessage::Text { dest, content } => {
        let mut replies = Vec::new();
        let reply = self.handle_text_message(src, dest, content).await;
        replies.push(reply);
        replies
      },
      ClientMessage::MText { dest, content } => {
          let mut replies = Vec::new();
          for client in dest {
            let reply = self.handle_text_message(src, client, content.clone()).await;
            replies.push(reply);
          }
          replies
      },
    }
  }

  #[cfg(feature = "federation")]
  async fn handle_server_message(&self, msg: ServerMessage) -> ServerReply {
    match msg {
      ServerMessage::Announce { route, clients } => {
        if route.len() == 0 {
          return ServerReply::EmptyRoute;
        }
      },
      ServerMessage::Message { .. } => {
        return ServerReply::Outgoing(Vec::new());
      },
    }
    ServerReply::Outgoing(Vec::new())
  }

  // return a route to the target server
  // bonus points if it is the shortest route
  #[cfg(feature = "federation")]
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>> {
    let routes = self.routes.read().await;

    // Find routes that contain the destination
    let mut valid_routes: Vec<&Vec<ServerId>> = routes
        .iter()
        .filter(|route| route.contains(&destination))
        .collect();

    // Find the shortest route
    valid_routes.sort_by_key(|route| route.len());

    // Return the shortest route
    valid_routes.first().cloned().cloned()
  }
}

impl Server {
  async fn handle_text_message(&self, src:ClientId, dest:ClientId, content:String) -> ClientReply {
    log::debug!("Handling text message from {} to {} : {}", src, dest, content);
    let mut clients = self.clients.write().await;
    let client = clients.iter_mut().find(|(id, _, _, _)| *id == dest);
    if client.is_none() {
      // Open write lock to add the message to the stored messages
      let mut stored = self.stored.write().await;
      stored.push((self.id, dest, content));
      ClientReply::Delayed
    } else {
      let (_, _, last_seen, mailbox) = client.unwrap();
      if mailbox.len() >= MAILBOX_SIZE {
        ClientReply::Error(ClientError::BoxFull(dest))
      } else {
        mailbox.push((src, content));
        ClientReply::Delivered
      }
    }
  }
}

#[cfg(test)]
mod test {
  use crate::testing::test_message_server;

  use super::*;

  #[test]
  fn tester() {
    test_message_server::<Server>();
  }
}
