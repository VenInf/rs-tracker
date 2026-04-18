use futures_util::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::io::{Error, ErrorKind};
use crate::{handshake};
use crate::pieces::{Bitfield, PieceDownloaded, PieceReq, PieceResponse, SharedDownloads, Task};
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
    pub peer_id: [u8; 20],
    framed_stream: Framed<TcpStream, LengthDelimitedCodec>,
    choked: Arc<AtomicBool>,
    peer_bitfield: Arc<Mutex<Bitfield>>,
    tasks_sender: mpsc::Sender<Task>,
    tasks_receiver: mpsc::Receiver<Task>,
    pub downloaded: mpsc::Sender<PieceDownloaded>,
    caught_block_pieces: Arc<Mutex<Vec<BlockPiece>>>,
    shared_downloads: Arc<SharedDownloads>
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

pub async fn send_download_pieces(sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>, task: &PieceReq) -> Result<(), Error> {
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
        total_pieces_length: u64,
        tasks_sender: mpsc::Sender<Task>,
        tasks_receiver: mpsc::Receiver<Task>, // TODO: add Have(piece) to tasks
        downloaded: mpsc::Sender<PieceDownloaded>,
        shared_downloads: Arc<SharedDownloads>
        ) -> Result<Self, Error> {
        // TODO: send our bitfield as well

        let addr = format!("{}:{}", address.0, address.1);
        println!("Attemt to connect at {}", addr);
        let mut stream = TcpStream::connect(addr).await?;

        let handshake = handshake::Handshake { info_hash, peer_id: my_peer_id };
        stream.write_all(&handshake.serialize()).await?;

        let mut response = [0u8; 68];
        stream.read_exact(&mut response).await?;

        let response_handshake = handshake::Handshake::parse(response)?;

        if response_handshake.info_hash != info_hash {
            return Err(Error::new(ErrorKind::InvalidData, "Wrong handshake hash response"));
        }
        
        Ok(Self {
            peer_id: response_handshake.peer_id,
            peer_bitfield: Arc::new(Mutex::new(Bitfield::new(total_pieces_length))),
            framed_stream: Framed::new(stream, LengthDelimitedCodec::new()), // We can use `LengthDelimitedCodec` since bittorrent protocol appends the length of the message at the start
            choked: AtomicBool::new(true).into(),
            caught_block_pieces: Arc::new(Mutex::new(vec![])),
            tasks_sender,
            tasks_receiver,
            downloaded,
            shared_downloads
        })
    }


    pub async fn interact_loop(self) -> Result<(), Error> {
        let (mut sink, mut stream) = self.framed_stream.split();
        let (accepted_piece_req_sender, mut accepted_piece_req_receiver) = mpsc::channel(5);
        
        let choked = self.choked.clone();
        let peer_bitfield = self.peer_bitfield.clone();

        let caught_block_pieces = self.caught_block_pieces.clone();

        // Check that the order is correct
        let bitfield = self.shared_downloads.bitfield.read().await.clone();
        let _ = send_message(&mut sink, TorrentTcpMessage::Bitfield(bitfield.bytes)).await;
        let _ = send_message(&mut sink, TorrentTcpMessage::Interested).await;
        let _ = send_message(&mut sink, TorrentTcpMessage::Unchoke).await;

        // Listener, also responses to peer requests
        tokio::spawn(async move {
            while let Ok(response) = read_message(&mut stream).await {
                match response {
                    TorrentTcpMessage::KeepAlive => continue,
                    TorrentTcpMessage::Bitfield(raw_bitfield) => {
                        let mut peer_bitfield = peer_bitfield.lock().await;
                        peer_bitfield.set_all(raw_bitfield);
                    }
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
                        if let Some(block) = self.shared_downloads.get_block(index, begin, length).await {
                            let response_task = Task::Response(PieceResponse { index, begin, block });
                            let _ = self.tasks_sender.send(response_task).await;
                        }
                    }
                    _ => return Err(Error::new(ErrorKind::InvalidData, "Unexpected TorrentTcpMessage received"))
                }
            }
            
            return Ok(());
        });
        
        let choked = self.choked.clone();
        let mut tasks_receiver = self.tasks_receiver;

        // Sender, for local requests only
        tokio::spawn(async move {
            loop {
                if choked.load(Ordering::Relaxed) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue; 
                }

                if let Some(task) = tasks_receiver.recv().await {
                    match task {
                        Task::Request(piece_req) => {
                            let peer_bitfield = self.peer_bitfield.lock().await;
                            if peer_bitfield.has(piece_req.piece_index) {
                                let _ = send_download_pieces(&mut sink, &piece_req).await;
                                let _ = accepted_piece_req_sender.send(piece_req).await;
                            }
                        }
                        Task::Response(PieceResponse { index, begin, block }) => {
                            let resonse_message = TorrentTcpMessage::Piece { index, begin, block };
                            send_message(&mut sink, resonse_message).await;
                        }

                    }
                }
            }
        });
        

        let caught_block_pieces = self.caught_block_pieces.clone();
        let downloaded = self.downloaded;

        // Bundler, creates DownloadedPiece's, break the connection if the peer misbehaves
        tokio::spawn(async move {
            while let Some(accepted_piece_req) = accepted_piece_req_receiver.recv().await {
                let timer = tokio::time::Instant::now();
                loop {
                    if timer.elapsed() > tokio::time::Duration::from_secs(60) {
                        // Max time per block
                        break;
                    }

                    let mut block_pieces_guard = caught_block_pieces.lock().await;
                    let mut block_pieces: Vec<BlockPiece> = block_pieces_guard.iter()
                                                                              .filter(|bp| bp.index == accepted_piece_req.piece_index)
                                                                              .cloned()
                                                                              .collect();

                    let current_length: u32 = block_pieces.iter().map(|bp| bp.block.len() as u32).sum();
                    if current_length >= accepted_piece_req.piece_length {
                        block_pieces.sort();

                        let piece_data: Vec<u8> = block_pieces.iter().flat_map(|piece| piece.block.iter()).copied().collect();
                        let piece_hash: [u8; 20] = Sha1::digest(&piece_data).into();
                        
                        if piece_hash == accepted_piece_req.piece_hash {
                            let _ = downloaded.send(PieceDownloaded { piece_data, piece_req: accepted_piece_req }).await;
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
            
            TorrentTcpMessage::Bitfield(bitfield) => {
                self.packet(5, &bitfield.as_slice())
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


