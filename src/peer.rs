use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::io::{Error, ErrorKind};
use crate::handshake;

pub struct ConnectedPeer {
    pub stream: TcpStream,
    pub peer_id: [u8; 20],
}

impl ConnectedPeer {
    pub async fn new(
        address: (String, u16),
        info_hash: [u8; 20],
        my_peer_id: [u8; 20],
    ) -> Result<Self, Error> {
        let addr = format!("{}:{}", address.0, address.1);
        let mut stream = TcpStream::connect(addr).await?;

        let handshake = handshake::Handshake { info_hash, peer_id: my_peer_id };
        stream.write_all(&handshake.serialize()).await?;

        let mut response = [0u8; 68];
        stream.read_exact(&mut response).await?;

        let response_handshake = handshake::Handshake::parse(response)?;

        if response_handshake.info_hash != info_hash {
            return Err(Error::new(ErrorKind::InvalidData, "Wrong hash response"));
        }

        Ok(Self {
            stream,
            peer_id: response_handshake.peer_id,
        })
    }

    pub async fn send_message(mut self, message: TorrentTcpMessage) -> Result<(), Error> {
        self.stream.write_all(message.serialize().as_slice()).await?;        
        Ok(())
    }

    pub async fn read_message(mut self) -> Result<TorrentTcpMessage, Error> {
        let length = self.stream.read_u32().await? as usize;

        if length == 0 {
            return Ok(TorrentTcpMessage::KeepAlive);
        }

        let id = self.stream.read_u8().await?;

        let payload_len = length - 1;
        let mut payload = vec![0u8; payload_len];
        self.stream.read_exact(&mut payload).await?;

        TorrentTcpMessage::parse(id, payload.as_slice())
    }    
}

pub enum TorrentTcpMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request { index: u32, begin: u32, length: u32 },
    Piece { index: u32, begin: u32, block: Vec<u8> },
    Cancel { index: u32, begin: u32, length: u32 },
}

impl TorrentTcpMessage {
    pub fn parse(id: u8, payload: &[u8]) -> Result<Self, Error> {
        match id {
            0 => Ok(TorrentTcpMessage::Choke),
            1 => Ok(TorrentTcpMessage::Unchoke),
            2 => Ok(TorrentTcpMessage::Interested),
            3 => Ok(TorrentTcpMessage::NotInterested),
            4 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().map_err(|_| "Invalid Have")?);
                Ok(TorrentTcpMessage::Have(index))
            }
            5 => Ok(TorrentTcpMessage::Bitfield(payload.to_vec())),
            6 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let length = u32::from_be_bytes(payload[8..12].try_into().unwrap());
                Ok(TorrentTcpMessage::Request { index, begin, length })
            }
            7 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let block = payload[8..].to_vec();
                Ok(TorrentTcpMessage::Piece { index, begin, block })
            }
            // Add other IDs as needed
            _ => Err(Error::new(ErrorKind::InvalidData, format!("Unknown Message ID: {}", id))),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
           TorrentTcpMessage::KeepAlive => vec![0; 4],
            
           TorrentTcpMessage::Choke => self.packet(0, &[]),
           TorrentTcpMessage::Unchoke => self.packet(1, &[]),
            
           TorrentTcpMessage::Have(index) => {
                self.packet(4, &index.to_be_bytes())
            }
            
           TorrentTcpMessage::Request { index, begin, length } => {
                let payload = [
                    index.to_be_bytes(),
                    begin.to_be_bytes(),
                    length.to_be_bytes(),
                ];
                self.packet(6, payload.concat().as_slice())
            }

           TorrentTcpMessage::Piece { index, begin, block } => {
                let header = [
                    index.to_be_bytes(),
                    begin.to_be_bytes(),
                ].concat();
                
                let payload = header.iter().copied().chain(block.iter().copied());
                
                let len = (9 + block.len()) as u32;
                len.to_be_bytes().into_iter()
                    .chain(std::iter::once(7))
                    .chain(payload)
                    .collect()
            }
            _ => todo!()
        }
    }

    fn packet(&self, id: u8, payload: &[u8]) -> Vec<u8> {
        let len = (payload.len() + 1) as u32;
        
        len.to_be_bytes().into_iter()
            .chain(std::iter::once(id))
            .chain(payload.iter().copied())
            .collect()
    }     
}


