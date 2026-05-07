use std::fmt;
use std::io::{Error, ErrorKind};

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum TorrentTcpMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
}

impl fmt::Display for TorrentTcpMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TorrentTcpMessage::KeepAlive => write!(f, "KeepAlive"),
            TorrentTcpMessage::Choke => write!(f, "Choke"),
            TorrentTcpMessage::Unchoke => write!(f, "Unchoke"),
            TorrentTcpMessage::Interested => write!(f, "Interested"),
            TorrentTcpMessage::NotInterested => write!(f, "NotInterested"),
            TorrentTcpMessage::Have(_) => write!(f, "Have"),
            TorrentTcpMessage::Bitfield(_) => write!(f, "Bitfield"),
            TorrentTcpMessage::Piece { .. } => write!(f, "Piece"),
            TorrentTcpMessage::Request { .. } => write!(f, "Request"),
            TorrentTcpMessage::Cancel { .. } => write!(f, "Cancel"),
        }
    }
}

impl TorrentTcpMessage {
    pub fn parse(id: &u8, payload: &[u8]) -> Result<Self, Error> {
        match id {
            0 => Ok(TorrentTcpMessage::Choke),
            1 => Ok(TorrentTcpMessage::Unchoke),
            2 => Ok(TorrentTcpMessage::Interested),
            3 => Ok(TorrentTcpMessage::NotInterested),
            4 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                Ok(TorrentTcpMessage::Have(index))
            }
            5 => Ok(TorrentTcpMessage::Bitfield(payload.to_vec())),
            6 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let length = u32::from_be_bytes(payload[8..12].try_into().unwrap());
                Ok(TorrentTcpMessage::Request {
                    index,
                    begin,
                    length,
                })
            }
            7 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let block = payload[8..].to_vec();
                Ok(TorrentTcpMessage::Piece {
                    index,
                    begin,
                    block,
                })
            }
            8 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let length = u32::from_be_bytes(payload[8..12].try_into().unwrap());
                Ok(TorrentTcpMessage::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            // Add other IDs as needed
            _ => Err(Error::new(
                ErrorKind::InvalidData,
                format!("Unknown Message ID: {}", id),
            )),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            TorrentTcpMessage::KeepAlive => vec![0; 4],

            TorrentTcpMessage::Choke => self.packet(0, &[]),
            TorrentTcpMessage::Unchoke => self.packet(1, &[]),
            TorrentTcpMessage::Interested => self.packet(2, &[]),
            TorrentTcpMessage::NotInterested => self.packet(3, &[]),

            TorrentTcpMessage::Have(index) => self.packet(4, &index.to_be_bytes()),

            TorrentTcpMessage::Bitfield(bitfield) => self.packet(5, &bitfield.as_slice()),

            TorrentTcpMessage::Request {
                index,
                begin,
                length,
            } => {
                let payload = [
                    index.to_be_bytes(),
                    begin.to_be_bytes(),
                    length.to_be_bytes(),
                ];
                self.packet(6, payload.concat().as_slice())
            }

            TorrentTcpMessage::Piece {
                index,
                begin,
                block,
            } => {
                let header = [index.to_be_bytes(), begin.to_be_bytes()].concat();

                let payload = header.iter().copied().chain(block.iter().copied());

                let len = (9 + block.len()) as u32;
                len.to_be_bytes()
                    .into_iter()
                    .chain(std::iter::once(7))
                    .chain(payload)
                    .collect()
            }

            TorrentTcpMessage::Cancel {
                index,
                begin,
                length,
            } => {
                let payload = [
                    index.to_be_bytes(),
                    begin.to_be_bytes(),
                    length.to_be_bytes(),
                ];
                self.packet(8, payload.concat().as_slice())
            }
        }
    }

    fn packet(&self, id: u8, payload: &[u8]) -> Vec<u8> {
        // We don't add length to the packets since this is handeled by tokio_util::codec::LengthDelimitedCodec
        let mut buf = Vec::with_capacity(1 + payload.len());
        buf.push(id);
        buf.extend_from_slice(payload);
        buf
    }
}
