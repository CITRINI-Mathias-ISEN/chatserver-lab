use std::{collections::HashMap, io::Write};

use byteorder::{LittleEndian, WriteBytesExt};
use uuid::Uuid;

use crate::messages::{
  AuthMessage, ClientError, ClientId, ClientMessage, ClientPollReply, ClientQuery, ClientReply,
  DelayedError, Sequence, ServerId, ServerMessage,
};

// look at the README.md for guidance on writing this function
// this function is used to encode all the "sizes" values that will appear after that
pub fn u128<W>(w: &mut W, m: u128) -> std::io::Result<()>
where
  W: Write,
{
  if m < 251 {
    w.write_u8(m as u8)?;
  } else if m < (1 << 16) {
    w.write_u8(251)?;
    w.write_u16::<LittleEndian>(m as u16)?;
  } else if m < (1 << 32) {
    w.write_u8(252)?;
    w.write_u32::<LittleEndian>(m as u32)?;
  } else if m < (1u128 << 64) {
    w.write_u8(253)?;
    w.write_u64::<LittleEndian>(m as u64)?;
  } else {
    w.write_u8(254)?;
    w.write_u128::<LittleEndian>(m)?;
  }

  Ok(())
}

/* UUIDs are 128bit values, but in the situation they are represented as [u8; 16]
  don't forget that arrays are encoded with their sizes first, and then their content
*/
fn uuid<W>(w: &mut W, m: &Uuid) -> std::io::Result<()>
where
  W: Write,
{
  let uuid_bytes = m.as_bytes();
  u128(w, uuid_bytes.len() as u128)?;
  w.write_all(uuid_bytes)?;
  Ok(())
}

// reuse uuid
pub fn clientid<W>(w: &mut W, m: &ClientId) -> std::io::Result<()>
where
  W: Write,
{
  uuid(w, &m.0)
}

// reuse uuid
pub fn serverid<W>(w: &mut W, m: &ServerId) -> std::io::Result<()>
where
  W: Write,
{
  uuid(w, &m.0)
}

// strings are encoded as the underlying bytes array
// so
//  1/ get the underlying bytes
//  2/ write the size (using u128)
//  3/ write the array
pub fn string<W>(w: &mut W, m: &str) -> std::io::Result<()>
where
  W: Write,
{
  // Convert the string to bytes
  let string_bytes = m.as_bytes();

  // Write the size of the array
  u128(w, string_bytes.len() as u128)?;

  // Write the array
  w.write_all(string_bytes)?;

  Ok(())
}

/* The following is VERY mechanical, and should be easy once the general principle is understood

* Structs

   Structs are encoded by encoding all fields, by order of declaration. For example:

   struct Test {
     a: u32,
     b: [u8; 3],
   }

   Test {a: 5, b: [1, 2, 3]} -> [5, 3, 1, 2, 3]  // the first '3' is the array length
   Test {a: 42, b: [5, 6, 7]} -> [42, 3, 5, 6, 7]

* Enums

   Enums are encoded by first writing a tag, corresponding to the variant index, and the the content of the variant.
   For example, if we have:

   enum Test { A, B(u32), C(u32, u32) };

   Test::A is encoded as [0]
   Test::B(8) is encoded as [1, 8]
   Test::C(3, 17) is encoded as [2, 3, 17]

 */

pub fn auth<W>(w: &mut W, m: &AuthMessage) -> std::io::Result<()>
where
  W: Write,
{
  match m {
    AuthMessage::Hello { user, nonce } => {
      u128(w, 0)?;
      clientid(w, user)?;
      nonce.iter().for_each(|b| u128(w, *b as u128).unwrap());
    }
    AuthMessage::Nonce { server, nonce } => {
      u128(w, 1)?;
      serverid(w, server)?;
      nonce.iter().for_each(|b| u128(w, *b as u128).unwrap());
    }
    AuthMessage::Auth { response } => {
      u128(w, 2)?;
      w.write_all(response)?
    }
  }
  Ok(())
}

pub fn server<W>(w: &mut W, m: &ServerMessage) -> std::io::Result<()>
where
  W: Write,
{
  match m {
    ServerMessage::Message(msg) => {
      // Encode Message variant
      u128(w, 1)?;

      clientid(w, &msg.src)?;
      serverid(w, &msg.srcsrv)?;

      u128(w, msg.dsts.len() as u128)?;
      for (client_id, server_id) in &msg.dsts {
        clientid(w, client_id)?;
        serverid(w, server_id)?;
      }

      string(w, &msg.content)?;
    }
    ServerMessage::Announce { route, clients } => {
      // Encode Announce variant
      u128(w, 0)?;

      u128(w, route.len() as u128)?; // Encode route size (u128
      for server_id in route {
        serverid(w, server_id)?;
      }

      // Encode clients
      u128(w, clients.len() as u128)?;
      for (client_id, client_name) in clients {
        clientid(w, client_id)?;
        string(w, client_name)?;
      }
    }
  }
  Ok(())
}

pub fn client<W>(w: &mut W, m: &ClientMessage) -> std::io::Result<()>
where
  W: Write,
{
  match m {
    ClientMessage::Text { dest, content } => {
      u128(w, 0)?;
      clientid(w, dest)?;
      string(w, content)
    }
    ClientMessage::MText { dest, content } => {
      u128(w, 1)?;
      u128(w, dest.len() as u128)?;
      for client_id in dest {
        clientid(w, client_id)?;
      }
      string(w, content)
    }
  }
}

pub fn client_replies<W>(w: &mut W, m: &[ClientReply]) -> std::io::Result<()>
where
  W: Write,
{
  for reply in m {
    match reply {
      ClientReply::Delivered => {
        u128(w, 0)?;
      }
      ClientReply::Error(err) => {
        u128(w, 1)?;
        match err {
          ClientError::WorkProofError => {
            u128(w, 0)?;
          }
          ClientError::UnknownClient => {
            u128(w, 1)?;
          }
          ClientError::SequenceError => {
            u128(w, 2)?;
          }
          ClientError::BoxFull(id) => {
            u128(w, 3)?;
            clientid(w, id)?;
          }
          ClientError::InternalError => {
            u128(w, 4)?;
          }
        }
      }
      ClientReply::Delayed => {
        u128(w, 2)?;
      }
      ClientReply::Transfer(id, msg) => {
        u128(w, 3)?;
        serverid(w, id)?;
        server(w, msg)?;
      }
    }
  }
  Ok(())
}

pub fn client_poll_reply<W>(w: &mut W, m: &ClientPollReply) -> std::io::Result<()>
where
  W: Write,
{
  match m {
    ClientPollReply::Message { src, content } => {
      u128(w, 0)?;
      clientid(w, src)?;
      string(w, content)?;
    }
    ClientPollReply::DelayedError(err) => {
      u128(w, 1)?;
      match err {
        DelayedError::UnknownRecipient(id) => {
          u128(w, 0)?;
          clientid(w, id)?;
        }
      }
    }
    ClientPollReply::Nothing => {
      u128(w, 2)?;
    }
  }
  Ok(())
}

// hashmaps are encoded by first writing the size (using u128), then each key and values
pub fn userlist<W>(w: &mut W, m: &HashMap<ClientId, String>) -> std::io::Result<()>
where
  W: Write,
{
  u128(w, m.len() as u128)?;
  for (client_id, client_name) in m {
    clientid(w, client_id)?;
    string(w, client_name)?;
  }
  Ok(())
}

pub fn client_query<W>(w: &mut W, m: &ClientQuery) -> std::io::Result<()>
where
  W: Write,
{
  match m {
    ClientQuery::Register(str) => {
      u128(w, 0)?;
      string(w, str)?;
    }
    ClientQuery::Message(msg) => {
      u128(w, 1)?;
      client(w, msg)?;
    }
    ClientQuery::Poll => {
      u128(w, 2)?;
    }
    ClientQuery::ListUsers => {
      u128(w, 3)?;
    }
  }
  Ok(())
}

pub fn sequence<X, W, ENC>(w: &mut W, m: &Sequence<X>, f: ENC) -> std::io::Result<()>
where
  W: Write,
  X: serde::Serialize,
  ENC: FnOnce(&mut W, &X) -> std::io::Result<()>,
{
  u128(w, m.seqid)?;
  clientid(w, &m.src)?;
  u128(w, m.workproof)?;
  f(w, &m.content)?;
  Ok(())
}
