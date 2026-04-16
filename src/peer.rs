use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::io::{Error, ErrorKind};
use crate::handshake;
use crate::pieces::{Piece, PieceDownloaded, PieceTask};
use sha1::{Digest, Sha1};
use futures_util::{StreamExt, SinkExt};

pub struct ConnectedPeer {
    // pub stream: TcpStream,
    pub framed_stream:  Framed<TcpStream, LengthDelimitedCodec>,
    pub chocked: bool,
    pub peer_id: [u8; 20],
}

impl ConnectedPeer {
    pub async fn new(
        address: (String, u16),
        info_hash: [u8; 20],
        my_peer_id: [u8; 20],
    ) -> Result<Self, Error> {
        let addr = format!("{}:{}", address.0, address.1);
        println!("Attemt to connect at {}", addr);
        let mut stream = TcpStream::connect(addr).await?;
        // We can use `LengthDelimitedCodec` since bittorrent protocol appends the length of the message at the start

        let handshake = handshake::Handshake { info_hash, peer_id: my_peer_id };
        stream.write_all(&handshake.serialize()).await?;

        let mut response = [0u8; 68];
        stream.read_exact(&mut response).await?;

        let response_handshake = handshake::Handshake::parse(response)?;

        if response_handshake.info_hash != info_hash {
            return Err(Error::new(ErrorKind::InvalidData, "Wrong handshake hash response"));
        }

        let framed_stream = Framed::new(stream, LengthDelimitedCodec::new());
        Ok(Self {
            framed_stream,
            chocked: true,
            peer_id: response_handshake.peer_id,
        })
    }

    pub async fn send_message(&mut self, message: TorrentTcpMessage) -> Result<(), Error> {
        self.framed_stream.send(message.serialize().into()).await?;      
        Ok(())
    }

    pub async fn read_message(&mut self) -> Result<TorrentTcpMessage, Error> {
        if let Some(frame) = self.framed_stream.next().await {
            let raw_message = frame?;

            if let Some((id, payload)) = raw_message.split_first() {
                return TorrentTcpMessage::parse(id, payload);
            } else {
                return Ok(TorrentTcpMessage::KeepAlive);
            }
        } else {
            Err(Error::new(ErrorKind::ConnectionAborted, "Peer stopped sending blocks"))
        }
    }

    pub async fn download_piece(&mut self, task: PieceTask) -> Result<PieceDownloaded, Error> {
        // Add the choke-unchoke here as well, discard all other messages

        let block_size = 16384;
        let mut piece_data = vec![0u8; task.piece_length as usize];
        
        for offset in (0..task.piece_length).step_by(block_size as usize) {
            let length = std::cmp::min(block_size, task.piece_length - offset);
            let req_message = TorrentTcpMessage::Request { index: task.piece_index, begin: offset, length };            
            self.send_message(req_message).await?;
            
            let response = self.read_message().await?;

            if let TorrentTcpMessage::Piece { index, begin, block } = response {
                if index != task.piece_index {
                    return Err(Error::new(ErrorKind::InvalidData, "Received block for wrong piece"));
                }

                println!("DEBUG: Caught a piece with index: {}, begin: {}", index, begin);
                
                let start = begin as usize;
                let end = start + block.len();
                
                if end <= piece_data.len() {
                    piece_data[start..end].copy_from_slice(&block);
                } else {
                    return Err(Error::new(ErrorKind::InvalidData, "Block exceeds piece length"));
                }
            } else {
                return Err(Error::new(ErrorKind::ConnectionAborted, "Peer stopped sending blocks"));
            }   
        }

        let piece_hash: [u8; 20] = Sha1::digest(&piece_data).into();
        
        if task.piece_hash == piece_hash {
            return Ok(PieceDownloaded {piece_data, piece_task: task});
        } else {
            return Err(Error::new(ErrorKind::InvalidData, "Failed to verify the received block hash"));
        }
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
                Ok(TorrentTcpMessage::Request { index, begin, length })
            }
            7 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let block = payload[8..].to_vec();
                Ok(TorrentTcpMessage::Piece { index, begin, block })
            }
            8 => {
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let length = u32::from_be_bytes(payload[8..12].try_into().unwrap());
                Ok(TorrentTcpMessage::Cancel { index, begin, length })
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
           TorrentTcpMessage::Interested => self.packet(2, &[]),
           TorrentTcpMessage::NotInterested => self.packet(3, &[]),
            
           TorrentTcpMessage::Have(index) => {
                self.packet(4, &index.to_be_bytes())
            }
            
            TorrentTcpMessage::Bitfield(bf) => {
                self.packet(5, &bf.as_slice())
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


           TorrentTcpMessage::Cancel { index, begin, length } => {
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
        let len = (payload.len() + 1) as u32;
        
        len.to_be_bytes().into_iter()
            .chain(std::iter::once(id))
            .chain(payload.iter().copied())
            .collect()
    }     
}


