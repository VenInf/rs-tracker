use futures_util::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::io::{Error, ErrorKind};
use crate::{handshake};
use crate::pieces::{PieceDownloaded, PieceTask};
use sha1::{Digest, Sha1};
use futures_util::{StreamExt, SinkExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc};

#[derive(Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct BlockPiece {
    index: u32,
    begin: u32,
    block: Vec<u8>
}

// TODO: Controller shouldn't think that a task is completed untill it receives a DownloadedTask!

pub struct ConnectedPeer {
    pub framed_stream:  Framed<TcpStream, LengthDelimitedCodec>,
    tasks: mpsc::Receiver<PieceTask>,
    pub downloaded: mpsc::Sender<PieceDownloaded>,
    pub choked: Arc<AtomicBool>,
    pub caught_block_pieces: Arc<Mutex<Vec<BlockPiece>>>,
    pub peer_id: [u8; 20],
}

pub async fn read_message(stream: &mut SplitStream<Framed<TcpStream, LengthDelimitedCodec>>) -> Result<TorrentTcpMessage, Error> {
    if let Some(frame) = stream.next().await {
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

pub async fn send_message(sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>, message: TorrentTcpMessage) -> Result<(), Error> {    
    sink.send(message.serialize().into()).await?;      
    Ok(())
}

pub async fn send_download_pieces(sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>, task: &PieceTask) -> Result<(), Error> {
    let block_size = 16384;
    
    for offset in (0..task.piece_length).step_by(block_size as usize) {
        let length = std::cmp::min(block_size, task.piece_length - offset);
        let req_message = TorrentTcpMessage::Request { index: task.piece_index, begin: offset, length };            
        send_message(sink, req_message).await?;
    }

    Ok(())
}    

impl ConnectedPeer {
    pub async fn new(
        address: (String, u16),
        info_hash: [u8; 20],
        my_peer_id: [u8; 20],
        tasks: mpsc::Receiver<PieceTask>,
        downloaded: mpsc::Sender<PieceDownloaded>,
        ) -> Result<Self, Error> {
        // TODO: add the catch for the bitfield, send ours

        let addr = format!("{}:{}", address.0, address.1);
        println!("Attemt to connect at {}", addr);
        let mut stream = TcpStream::connect(addr).await?;
        // We can use `LengthDelimitedCodec` sinke bittorrent protocol appends the length of the message at the start

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
            choked: AtomicBool::new(true).into(),
            caught_block_pieces: Arc::new(Mutex::new(vec![])),        
            peer_id: response_handshake.peer_id,
            tasks,
            downloaded,
        })
    }


    pub async fn interact_loop(self) -> Result<(), Error> {
        let (mut sink, mut stream) = self.framed_stream.split();
        let (accepted_tasks_sender, mut accepted_tasks_receiver) = mpsc::channel(5);
        
        let choked = self.choked.clone();
        let caught_block_pieces = self.caught_block_pieces.clone();

        // Listener, also responses to peer requests
        tokio::spawn(async move {
            while let Ok(response) = read_message(&mut stream).await {
                match response {
                    TorrentTcpMessage::KeepAlive => continue,
                    TorrentTcpMessage::Choke => {
                        choked.store(true, Ordering::Relaxed);
                        continue;
                    }
                    TorrentTcpMessage::Unchoke => {
                        choked.store(false, Ordering::Relaxed);
                        continue;
                    }
                    TorrentTcpMessage::Piece { index, begin, block } => {
                        let mut pieces = caught_block_pieces.lock().await;
                        let bp = BlockPiece { index, begin, block };
                        // No duplicates stored
                        if !pieces.contains(&bp) {
                            pieces.push(bp);
                        }
                    } 
                    TorrentTcpMessage::Request { index, begin, length } => {
                        // TODO: send back a packet of local data
                    }
                    _ => return Err(Error::new(ErrorKind::InvalidData, "Unexpected TorrentTcpMessage received"))
                }
            }
            
            return Ok(());
        });
        
        let choked = self.choked.clone();
        let mut tasks = self.tasks;

        // Sender, for local requests only
        tokio::spawn(async move {
            loop {
                if choked.load(Ordering::Relaxed) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue; 
                }

                if let Some(task) = tasks.recv().await {
                    let _ = send_download_pieces(&mut sink, &task).await;
                    let _ = accepted_tasks_sender.send(task).await;
                }
            }
        });
        

        let caught_block_pieces = self.caught_block_pieces.clone();
        let downloaded = self.downloaded;

        // Bundler, creates DownloadedPiece's, break the connection if the peer misbehaves
        tokio::spawn(async move {
            while let Some(accepted_task) = accepted_tasks_receiver.recv().await {
                let timer = tokio::time::Instant::now();
                loop {
                    if timer.elapsed() > tokio::time::Duration::from_secs(60) {
                        // Max time per block
                        break;
                    }

                    let mut block_pieces_guard = caught_block_pieces.lock().await;
                    let mut block_pieces: Vec<BlockPiece> = block_pieces_guard.iter()
                                                                              .filter(|bp| bp.index == accepted_task.piece_index)
                                                                              .cloned()
                                                                              .collect();

                    let current_length: u32 = block_pieces.iter().map(|bp| bp.block.len() as u32).sum();
                    if current_length >= accepted_task.piece_length {
                        block_pieces.sort();

                        let piece_data: Vec<u8> = block_pieces.iter().flat_map(|piece| piece.block.iter()).copied().collect();
                        let piece_hash: [u8; 20] = Sha1::digest(&piece_data).into();
                        
                        if piece_hash == accepted_task.piece_hash {
                            let _ = downloaded.send(PieceDownloaded { piece_data, piece_task: accepted_task }).await;
                            block_pieces_guard.retain(|bp| !block_pieces.contains(&bp));
                            break;
                        }
                    }
                    
                    drop(block_pieces_guard);
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                };
            }
        });
        return Ok(());
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


