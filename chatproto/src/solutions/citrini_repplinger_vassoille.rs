use crate::messages::ServerMessage;
use async_std::sync::RwLock;
use async_trait::async_trait;
use std::collections::HashMap;
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
use crate::messages::{Outgoing, ServerReply};

// this structure will contain the data you need to track in your server
// this will include things like delivered messages, clients last seen sequence number, etc.
pub struct Server {
  id: ServerId,
  clients: RwLock<Vec<Client>>,       // Clients registered on the server
  routes: RwLock<Vec<Vec<ServerId>>>, // Routes to each server
  stored: RwLock<Vec<Stored>>,        // Stored messages
  remote_clients: RwLock<HashMap<ClientId, ClientInfo>>, // Remote clients
}

pub struct Stored {
  src: ClientId,   // Source client
  dst: ClientId,   // Destination client
  content: String, // Content of the message
}

pub struct Client {
  id: ClientId,                     // Client id
  name: String,                     // Client name
  last_seen: u64,                   // Last seen sequence number
  mailbox: Vec<(ClientId, String)>, // Mailbox of the client
}

pub struct ClientInfo {
  name: String,     // Client name
  server: ServerId, // Server id
}

#[async_trait]
impl MessageServer for Server {
  const GROUP_NAME: &'static str = "CITRINI Mathias, REPPLINGER Julien, VASSOILLE Rémi";

  fn new(id: ServerId) -> Self {
    Self {
      id,
      clients: Default::default(),
      routes: Default::default(),
      stored: Default::default(),
      remote_clients: Default::default(),
    }
  }

  // note: you need to roll a Uuid, and then convert it into a ClientId
  // Uuid::new_v4() will generate such a value
  // you will most likely have to edit the Server struct as as to store information about the client
  async fn register_local_client(&self, name: String) -> ClientId {
    let uuid = Uuid::new_v4();
    let client_id = ClientId::from(uuid);
    log::info!("Registering client {} with id {}", name, client_id);
    let mut clients = self.clients.write().await;
    clients.push(Client {
      id: client_id,
      name,
      last_seen: 0,
      mailbox: Vec::new(),
    });
    drop(clients);
    return client_id;
  }

  /*
   implementation notes:
   * the workproof should be checked first
    * the nonce is in sequence.src and should be converted with (&sequence.src).into()
   * then, if the client is known, its last seen sequence number must be verified (and updated)
  */
  async fn list_users(&self) -> HashMap<ClientId, String> {
    let mut users = HashMap::new();
    // Local
    let clients = self.clients.read().await;
    for client in clients.iter() {
      users.insert(client.id, client.name.clone());
    }
    drop(clients);
    // Remote
    let remote_clients = self.remote_clients.read().await;
    for (client_id, client_info) in remote_clients.iter() {
      users.insert(client_id.clone(), client_info.name.clone());
    }
    drop(remote_clients);
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
  async fn handle_sequenced_message<A: Send>(
    &self,
    sequence: Sequence<A>,
  ) -> Result<A, ClientError> {
    log::debug!(
      "Handling sequenced message from {} with seqid {}",
      sequence.src,
      sequence.seqid
    );

    // Check workproof
    if !verify_workproof(
      (&sequence.src).into(),
      sequence.workproof,
      WORKPROOF_STRENGTH,
    ) {
      log::debug!("Workproof error");
      return Err(ClientError::WorkProofError);
    }

    // Check if client is known
    let mut clients = self.clients.write().await;
    let client = clients.iter().find(|client| client.id == sequence.src);
    if let Some(client) = client {
      log::debug!("Message from known client : {}", client.name);

      // Check if sequence number is increasing
      log::debug!("received: {}", sequence.seqid);
      if client.last_seen >= (sequence.seqid as u64) {
        log::debug!(
          "Sequence error current: {}, received: {}",
          client.last_seen,
          sequence.seqid
        );
        return Err(ClientError::SequenceError);
      }

      if let Some(client) = clients.iter_mut().find(|client| client.id == sequence.src) {
        client.last_seen = sequence.seqid as u64;
      } else {
        log::debug!("Unknown client");
        return Err(ClientError::UnknownClient);
      }
    } else {
      log::debug!("Unknown client");
      return Err(ClientError::UnknownClient);
    }
    return Ok(sequence.content);
  }

  /* for the given client, return the next message or error if available */
  async fn client_poll(&self, client_id: ClientId) -> ClientPollReply {
    log::debug!("Polling client {:?}", client_id);

    // Opening the write lock to remove the message from the mailbox
    let mut clients = self.clients.write().await;
    let client_elem = clients.iter_mut().find(|client| client.id == client_id);

    // If the client is known
    if let Some(client) = client_elem {
      // If the mailbox is empty
      if client.mailbox.is_empty() {
        return ClientPollReply::Nothing;
      }

      // Get the first message
      let (src, msg) = client.mailbox.remove(0);
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
      }
      ClientMessage::MText { dest, content } => {
        let mut replies = Vec::new();
        for client in dest {
          let reply = self.handle_text_message(src, client, content.clone()).await;
          replies.push(reply);
        }
        replies
      }
    }
  }

  #[cfg(feature = "federation")]
  async fn handle_server_message(&self, msg: ServerMessage) -> ServerReply {
    return match msg {
      /* -- Announce -- */
      ServerMessage::Announce { route, clients } => {
        if route.len() == 0 {
          return ServerReply::EmptyRoute;
        }
        // Store the remote clients
        let mut remote_clients = self.remote_clients.write().await;
        for (client_id, string) in clients {
          remote_clients.insert(
            client_id,
            ClientInfo {
              name: string,
              server: route.last().unwrap().clone(),
            },
          );
        }
        // Store the route
        let mut routes = self.routes.write().await;
        routes.push(route);
        drop(routes);
        // Check if there are messages waiting for the remote clients, in this case, send server messages
        let mut replies = Vec::new();
        let stored = self.stored.write().await;
        for message in stored.iter() {
          if let Some(client_info) = remote_clients.get(&message.dst) {
            if let Some(route) = self.route_to(client_info.server).await {
              let dst = route.last().unwrap().clone();
              let next_hop = route.get(1).unwrap().clone();
              replies.push(Outgoing {
                nexthop: next_hop,
                message: FullyQualifiedMessage {
                  src: message.src,
                  srcsrv: self.id,
                  dsts: vec![(message.dst, dst)],
                  content: message.content.clone(),
                },
              });
            }
          }
        }
        drop(stored);
        ServerReply::Outgoing(replies)
      }
      /* -- Message -- */
      ServerMessage::Message(msg) => {
        let next_server = self
          .route_to(msg.dsts.first().unwrap().1)
          .await
          .unwrap()
          .last()
          .unwrap()
          .clone();
        ServerReply::Outgoing(vec![Outgoing {
          nexthop: next_server,
          message: FullyQualifiedMessage {
            src: msg.src,
            srcsrv: msg.srcsrv,
            dsts: msg.dsts,
            content: msg.content,
          },
        }])
      }
    };
  }

  // return a route to the target server
  // bonus points if it is the shortest route
  #[cfg(feature = "federation")]
  async fn route_to(&self, destination: ServerId) -> Option<Vec<ServerId>> {
    let routes = self.routes.read().await;

    // reverse the routes to have the destination at the end and add the current server at the beginning
    let routes_m: Vec<Vec<ServerId>> = routes
      .iter()
      .filter(|route| route.contains(&destination))
      .map(|route| {
        let mut route = route.clone();
        route.reverse();

        route.insert(0, self.id);

        route
      })
      .collect();
    routes_m.first().cloned()
  }
}

impl Server {
  async fn handle_text_message(
    &self,
    src: ClientId,
    dest: ClientId,
    content: String,
  ) -> ClientReply {
    log::debug!(
      "Handling text message from {} to {} : {}",
      src,
      dest,
      content
    );

    let mut clients = self.clients.write().await;
    let client = clients.iter_mut().find(|client| client.id == dest);
    let result = match client {
      None => {
        let maybe_id: Option<ServerId> = self.get_remote_client_server(dest).await;
        if let Some(id) = maybe_id {
          let next_server = self.route_to(id).await.unwrap().last().unwrap().clone();
          ClientReply::Transfer(
            id,
            ServerMessage::Message(FullyQualifiedMessage {
              src,
              srcsrv: self.id,
              dsts: vec![(dest, next_server)],
              content,
            }),
          )
        } else {
          let mut stored = self.stored.write().await;
          stored.push(Stored {
            src,
            dst: dest,
            content,
          });
          ClientReply::Delayed
        }
      }
      Some(client) => {
        if client.mailbox.len() >= MAILBOX_SIZE {
          ClientReply::Error(ClientError::BoxFull(dest))
        } else {
          client.mailbox.push((src, content));
          ClientReply::Delivered
        }
      }
    };

    result
  }

  async fn get_remote_client_server(&self, client_id: ClientId) -> Option<ServerId> {
    let remote_clients = self.remote_clients.read().await;
    remote_clients
      .get(&client_id)
      .map(|client_info| client_info.server)
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
