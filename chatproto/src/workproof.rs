use byteorder::{LittleEndian, WriteBytesExt};
use crypto_hash::{digest, Algorithm, Hasher};

const LOOPS: usize = 16;

fn hashing(nonce: u128, start: u128) -> Vec<u8> {
  let mut hasher = Hasher::new(Algorithm::SHA1);
  hasher.write_u128::<LittleEndian>(nonce).unwrap();
  hasher.write_u128::<LittleEndian>(start).unwrap();
  let mut cur = hasher.finish();

  for _ in 1..LOOPS {
    cur = digest(Algorithm::SHA1, &cur);
  }
  cur
}

fn get_leading(bytes: &[u8]) -> u32 {
  let mut zeros = 0;
  for &byte in bytes {
    zeros += byte.leading_zeros();
    if byte != 0 {
      break;
    }
  }

  zeros
}

pub fn verify_workproof(nonce: u128, start: u128, strength: u32) -> bool {
  let hashed = hashing(nonce, start);
  get_leading(&hashed) >= strength
}

pub fn gen_workproof(nonce: u128, strength: u32, limit: u128) -> Option<u128> {
  (0..limit).find(|&start| verify_workproof(nonce, start, strength))
}

#[cfg(test)]
mod test {
  use super::*;

  #[test]
  fn leading_zeroes() {
    assert_eq!(get_leading(&[]), 0);
    assert_eq!(get_leading(&[0]), 8);
    assert_eq!(get_leading(&[1]), 7);
    assert_eq!(get_leading(&[128]), 0);
    assert_eq!(get_leading(&[64]), 1);
    assert_eq!(get_leading(&[0, 0, 64, 12]), 17);
  }

  #[test]
  fn find_workproof_easy() {
    assert_eq!(gen_workproof(161566988, 8, u128::MAX), Some(186));
  }

  #[test]
  fn find_workproof_impossible() {
    assert_eq!(gen_workproof(161566988, 8, 100), None);
  }
}
