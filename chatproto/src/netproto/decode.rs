use std::{collections::HashMap, io::Read};

use byteorder::{LittleEndian, ReadBytesExt};
use uuid::Uuid;

use crate::messages::{AuthMessage, ClientError, ClientId, ClientMessage, ClientPollReply, ClientQuery, ClientReply, DelayedError, FullyQualifiedMessage, Sequence, ServerId, ServerMessage};

// look at the README.md for guidance on writing this function
pub fn u128<R: Read>(rd: &mut R) -> anyhow::Result<u128> {
  let first_byte = rd.read_u8()?;
  Ok(match first_byte {
    0..=250 => u128::from(first_byte),
    251 => u128::from(rd.read_u16::<LittleEndian>()?),
    252 => u128::from(rd.read_u32::<LittleEndian>()?),
    253 => u128::from(rd.read_u64::<LittleEndian>()?),
    254 => rd.read_u128::<LittleEndian>()?,
    _ => panic!("Invalid first byte"),
  })
}

fn uuid<R: Read>(rd: &mut R) -> anyhow::Result<Uuid> {
  // Read the length of the UUID bytes
  let uuid_len = u128(rd)?;

  // Read the UUID bytes
  let mut uuid_bytes = vec![0u8; uuid_len as usize];
  rd.read_exact(&mut uuid_bytes)?;

  // Convert vec<u8> to vec<Bytes>
    let uuid_bytes = uuid_bytes.into_iter().map(|b| b.into()).collect::<Vec<_>>();

  // Convert vec<Bytes> to Uuid
  Ok(Uuid::from_slice(&uuid_bytes).unwrap())
}

// hint: reuse uuid
pub fn clientid<R: Read>(rd: &mut R) -> anyhow::Result<ClientId> {
  Ok(ClientId(uuid(rd)?))
}

// hint: reuse uuid
pub fn serverid<R: Read>(rd: &mut R) -> anyhow::Result<ServerId> {
  Ok(ServerId(uuid(rd)?))
}

pub fn string<R: Read>(rd: &mut R) -> anyhow::Result<String> {

    // Read the size of the array
    let size = u128(rd)?;

    // Read the array
    let mut string_bytes = vec![0u8; size as usize];
    rd.read_exact(&mut string_bytes)?;

    // Convert bytes to String
    let string_value = String::from_utf8_lossy(&string_bytes).into_owned();

    Ok(string_value)
}

pub fn auth<R: Read>(rd: &mut R) -> anyhow::Result<AuthMessage> {
  let tag = rd.read_u8()?;
  match tag {
    0 => {
      // Decode Hello variant
      let user = clientid(rd)?;
      let mut nonce = [0u8; 8];
      rd.read_exact(&mut nonce)?;
      Ok(AuthMessage::Hello { user, nonce })
    }
    1 => {
      // Decode Welcome variant
      let server = serverid(rd)?;
      let mut nonce = [0u8; 8];
      rd.read_exact(&mut nonce)?;
      Ok(AuthMessage::Nonce { server, nonce })
    }
    2 => {
      // Read the AuthMessage::Auth { response } => { u8;16
      let mut response = [0u8; 16];
      rd.read_exact(&mut response)?;
        Ok(AuthMessage::Auth { response })
    }
    _ => panic!("Invalid tag"),
  }
}

pub fn client<R: Read>(rd: &mut R) -> anyhow::Result<ClientMessage> {
  let tag = rd.read_u8()?;
  match tag {
    0 => {
      // Decode Text variant
      let dest = clientid(rd)?;
      let content = string(rd)?;
      Ok(ClientMessage::Text { dest, content })
    }
    1 => {
      // Decode MText variant
      let dest_size = u128(rd)? as usize;
      let dest = (0..dest_size).map(|_| clientid(rd)).collect::<Result<_, _>>()?;
      let content = string(rd)?;
      Ok(ClientMessage::MText { dest, content })
    }
    _ => panic!("Invalid tag"),
  }
}

pub fn client_replies<R: Read>(rd: &mut R) -> anyhow::Result<Vec<ClientReply>> {
  let mut replies = Vec::new();

  loop {
    let tag = u128(rd)? as usize;
    match tag {
      0 => replies.push(ClientReply::Delivered),
      1 => {
        let err_tag = u128(rd)? as usize;
        let err = match err_tag {
          0 => ClientError::WorkProofError,
          1 => ClientError::UnknownClient,
          2 => ClientError::SequenceError,
          3 => ClientError::BoxFull(clientid(rd)?),
          4 => ClientError::InternalError,
          _ => panic!("Invalid error tag"),
        };
        replies.push(ClientReply::Error(err));
      }
      2 => replies.push(ClientReply::Delayed),
      3 => {
        let server_id = serverid(rd)?;
        let server_msg = server(rd)?;
        replies.push(ClientReply::Transfer(server_id, server_msg));
      }
      _ => panic!("Invalid tag"),
    }
  }
}

pub fn client_poll_reply<R: Read>(rd: &mut R) -> anyhow::Result<ClientPollReply> {
  let tag = u128(rd)? as usize;
  match tag {
    0 => {
      let src = clientid(rd)?;
      let content = string(rd)?;
      Ok(ClientPollReply::Message { src, content })
    }
    1 => {
      let err_tag = u128(rd)? as usize;
      let err = match err_tag {
        0 => DelayedError::UnknownRecipient(clientid(rd)?),
        // Add other error variants if needed
        _ => panic!("Invalid error tag"),
      };
      Ok(ClientPollReply::DelayedError(err))
    }
    2 => Ok(ClientPollReply::Nothing),
    _ => panic!("Invalid tag"),
  }
}

pub fn server<R: Read>(rd: &mut R) -> anyhow::Result<ServerMessage> {
  let tag = rd.read_u8()?;
  match tag {
    0 => {
      // Decode Announce variant
      let route_size = u128(rd)? as usize;
      let mut route = Vec::with_capacity(route_size);
      for _ in 0..route_size {
        route.push(serverid(rd)?);
      }

      let clients_size = u128(rd)? as usize;
      let mut clients = HashMap::with_capacity(clients_size);
      for _ in 0..clients_size {
        let client_id = clientid(rd)?;
        let client_name = string(rd)?;
        clients.insert(client_id, client_name);
      }

      Ok(ServerMessage::Announce { route, clients })
    }
    1 => {
      // Decode Message variant
      let src = clientid(rd)?;
      let srcsrv = serverid(rd)?;

      let dsts_size = u128(rd)? as usize;
      let mut dsts = Vec::with_capacity(dsts_size);
        for _ in 0..dsts_size {
            let client_id = clientid(rd)?;
            let server_id = serverid(rd)?;
            dsts.push((client_id, server_id));
        }

      let content = string(rd)?;

      let msg = FullyQualifiedMessage {
        src,
        srcsrv,
        dsts,
        content,
      };

        Ok(ServerMessage::Message(msg))
    }
    _ => panic!("Invalid tag"),
  }
}

pub fn userlist<R: Read>(rd: &mut R) -> anyhow::Result<HashMap<ClientId, String>> {
    let mut clients = HashMap::new();

    let clients_size = u128(rd)? as usize;
    for _ in 0..clients_size {
        let client_id = clientid(rd)?;
        let client_name = string(rd)?;
        clients.insert(client_id, client_name);
    }

    Ok(clients)
}

pub fn client_query<R: Read>(rd: &mut R) -> anyhow::Result<ClientQuery> {
  let tag = u128(rd)? as usize;
  match tag {
    0 => {
      let string = string(rd)?;
      Ok(ClientQuery::Register(string))
    }
    1 => {
      let msg = client(rd)?;
      Ok(ClientQuery::Message(msg))
    }
    2 => Ok(ClientQuery::Poll),
    3 => Ok(ClientQuery::ListUsers),
    _ => panic!("Invalid tag"),
  }
}

pub fn sequence<X, R: Read, DEC>(rd: &mut R, d: DEC) -> anyhow::Result<Sequence<X>>
where
  DEC: FnOnce(&mut R) -> anyhow::Result<X>,
{
    let seqid = u128(rd)?;
    let src = clientid(rd)?;
    let workproof = u128(rd)?;
    let content = d(rd)?;
    Ok(Sequence {
        seqid,
        src,
        workproof,
        content,
    })
}
