use std::io::{Error, ErrorKind};

pub struct Handshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handshake {
    pub fn parse(buf: [u8; 68]) -> Result<Self, Error> {
        if buf[0] != 19 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Invalid protocol length",
            ));
        }

        if &buf[1..20] != b"BitTorrent protocol" {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Invalid protocol string",
            ));
        }

        let mut info_hash = [0u8; 20];
        info_hash.copy_from_slice(&buf[28..48]);

        let mut peer_id = [0u8; 20];
        peer_id.copy_from_slice(&buf[48..68]);

        Ok(Self { info_hash, peer_id })
    }

    pub fn serialize(&self) -> [u8; 68] {
        let mut res = [0u8; 68];

        res[0] = 19;
        res[1..20].copy_from_slice(b"BitTorrent protocol");

        res[28..48].copy_from_slice(&self.info_hash);
        res[48..68].copy_from_slice(&self.peer_id);

        res
    }
}
