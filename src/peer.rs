use crate::handshake;
use crate::pieces::{
    Bitfield, PieceDownloaded, PieceRequest, PieceResponse, SharedDownloads, Task,
};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use sha1::{Digest, Sha1};
use std::fmt;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::time::{Duration, timeout};
use tokio_util::bytes::Bytes;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub struct ConnectedPeer {
    pub peer_id: [u8; 20],
    pub peer_bitfield_arc: Arc<Mutex<Bitfield>>,
    sent_have_bitfield_arc: Arc<Mutex<Bitfield>>,
    framed_stream: Framed<TcpStream, LengthDelimitedCodec>,
    caught_piece_responses_arc: Arc<Mutex<Vec<PieceResponse>>>,
    choked_sender: watch::Sender<bool>,
    choked_receiver: watch::Receiver<bool>,
    tasks_sender: mpsc::Sender<Task>,
    tasks_receiver: mpsc::Receiver<Task>,
    shared_downloads_arc: Arc<SharedDownloads>,
    request_permits_sender: mpsc::Sender<()>,
    request_permits_receiver: mpsc::Receiver<()>,
}

impl ConnectedPeer {
    pub async fn new(
        address: (String, u16),
        info_hash: [u8; 20],
        my_peer_id: [u8; 20],
        total_amount_of_pieces: u64,
        tasks_sender: mpsc::Sender<Task>,
        tasks_receiver: mpsc::Receiver<Task>,
        shared_downloads_arc: Arc<SharedDownloads>,
    ) -> Result<Self, Error> {
        let addr = format!("{}:{}", address.0, address.1);
        tracing::info!("Attemt to connect at {}", addr);
        let mut stream = TcpStream::connect(addr).await?;

        let handshake = handshake::Handshake {
            info_hash,
            peer_id: my_peer_id,
        };
        stream.write_all(&handshake.serialize()).await?;

        let mut response = [0u8; 68];
        stream.read_exact(&mut response).await?;

        let response_handshake = handshake::Handshake::parse(response)?;

        if response_handshake.info_hash != info_hash {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "Wrong handshake hash response",
            ));
        }

        let (choked_sender, choked_receiver) = watch::channel(true);
        // We can use `LengthDelimitedCodec` since bittorrent protocol appends the length of the message at the start
        let framed_stream = Framed::new(stream, LengthDelimitedCodec::new());
        let (request_permits_sender, request_permits_receiver) =
            tokio::sync::mpsc::channel::<()>(10);
        for _ in 0..request_permits_sender.capacity() {
            request_permits_sender.send(()).await.unwrap();
        }

        Ok(Self {
            peer_id: response_handshake.peer_id,
            peer_bitfield_arc: Arc::new(Mutex::new(Bitfield::new(total_amount_of_pieces))),
            sent_have_bitfield_arc: Arc::new(Mutex::new(Bitfield::new(total_amount_of_pieces))),
            caught_piece_responses_arc: Arc::new(Mutex::new(vec![])),
            framed_stream,
            choked_sender,
            choked_receiver,
            tasks_sender,
            tasks_receiver,
            shared_downloads_arc,
            request_permits_sender,
            request_permits_receiver,
        })
    }

    pub async fn read_message(
        stream: &mut SplitStream<Framed<TcpStream, LengthDelimitedCodec>>,
        request_permits_sender: &mut mpsc::Sender<()>,
    ) -> Result<TorrentTcpMessage, Error> {
        if let Some(frame) = stream.next().await {
            let raw_message = frame?;

            if let Some((id, payload)) = raw_message.split_first() {
                let message_res = TorrentTcpMessage::parse(id, payload);

                if let Ok(message) = message_res {
                    if let TorrentTcpMessage::Piece { .. } = message {
                        tracing::debug!("Send more request permits");
                        let _ = request_permits_sender.send(()).await;
                    }
                    return Ok(message);
                }

                return TorrentTcpMessage::parse(id, payload);
            } else {
                return Ok(TorrentTcpMessage::KeepAlive);
            }
        } else {
            Err(Error::new(
                ErrorKind::ConnectionAborted,
                "Peer stopped sending blocks",
            ))
        }
    }

    pub async fn send_message(
        sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>,
        message: TorrentTcpMessage,
    ) -> Result<(), Error> {
        sink.send(message.serialize().into()).await?;
        match message {
            TorrentTcpMessage::Bitfield(_) => tracing::info!("Sent message {}", message),
            TorrentTcpMessage::Interested => tracing::info!("Sent message {}", message),
            TorrentTcpMessage::NotInterested => tracing::info!("Sent message {}", message),
            TorrentTcpMessage::Choke => tracing::info!("Sent message {}", message),
            TorrentTcpMessage::Unchoke => tracing::info!("Sent message {}", message),
            TorrentTcpMessage::Piece { index, .. } => {
                tracing::info!("Sent message {}, index {}", message, index)
            }

            _ => tracing::debug!("Sent message {}", message),
        }
        Ok(())
    }

    pub async fn send_request_message(
        sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>,
        choked_receiver: &mut watch::Receiver<bool>,
        request_permits_receiver: &mut mpsc::Receiver<()>,
        message: TorrentTcpMessage,
    ) -> Result<(), Error> {
        if let TorrentTcpMessage::Request {
            index,
            begin,
            length,
        } = message
        {
            request_permits_receiver.recv().await;
            choked_receiver
                .wait_for(|&is_choked| !is_choked)
                .await
                .map_err(|_| Error::new(ErrorKind::ConnectionAborted, "Watch sender dropped"))?;
            tracing::debug!(
                "Passed call with Request (index, begin, length) : ({}, {}, {})",
                index,
                begin,
                length
            );
        }

        Self::send_message(sink, message).await
    }

    pub async fn send_download_pieces(
        sink: &mut SplitSink<Framed<TcpStream, LengthDelimitedCodec>, Bytes>,
        choked_receiver: &mut watch::Receiver<bool>,
        request_permits_receiver: &mut mpsc::Receiver<()>,
        task: PieceRequest,
    ) -> Result<(), Error> {
        let block_size = 16384;

        for offset in (0..task.piece_length).step_by(block_size as usize) {
            let length = std::cmp::min(block_size, task.piece_length - offset);
            let req_message = TorrentTcpMessage::Request {
                index: task.piece_index,
                begin: offset,
                length,
            };
            Self::send_request_message(
                sink,
                choked_receiver,
                request_permits_receiver,
                req_message,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn interact_loop(self) -> Result<(), Error> {
        let (mut sink, mut stream) = self.framed_stream.split();

        let shared_downloads = self.shared_downloads_arc.clone();

        let bitfield = shared_downloads.bitfield.read().await.clone();
        Self::send_message(&mut sink, TorrentTcpMessage::Bitfield(bitfield.bytes)).await?;
        Self::send_message(&mut sink, TorrentTcpMessage::Interested).await?;
        Self::send_message(&mut sink, TorrentTcpMessage::Unchoke).await?;

        let (accepted_piece_req_sender, mut accepted_piece_req_receiver) = mpsc::channel(3);

        let peer_bitfield = self.peer_bitfield_arc.clone();
        let sent_have_bitfield = self.sent_have_bitfield_arc.clone();

        let caught_piece_responses = self.caught_piece_responses_arc.clone();

        let mut request_permits_sender = self.request_permits_sender.clone();
        let mut choked_receiver = self.choked_receiver;
        let mut request_permits_receiver = self.request_permits_receiver;
        let tasks_sender = self.tasks_sender.clone();

        // Listener, also responses to peer requests
        tokio::task::Builder::new()
            .name(&format!(
                "Listener for: {:?}",
                String::from_utf8_lossy(&self.peer_id)
            ))
            .spawn(async move {
                loop {
                    let inactivity_time = Duration::from_secs(5);
                    let message_result = timeout(
                        inactivity_time,
                        Self::read_message(&mut stream, &mut request_permits_sender),
                    )
                    .await;
                    let response: TorrentTcpMessage = message_result??;

                    match response {
                        TorrentTcpMessage::KeepAlive => continue,
                        TorrentTcpMessage::Bitfield(raw_bitfield) => {
                            tracing::info!(
                                "Caught Bitfield from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            let mut peer_bitfield_guard = peer_bitfield.lock().await;
                            peer_bitfield_guard.set_all(&raw_bitfield);

                            tracing::info!(
                                "Peer {} has {} pieces our of {} pieces",
                                String::from_utf8_lossy(&self.peer_id),
                                peer_bitfield_guard.total_set(),
                                peer_bitfield_guard.total()
                            );
                            drop(peer_bitfield_guard);

                            let mut sent_have_bitfield = sent_have_bitfield.lock().await;
                            sent_have_bitfield.set_all(&raw_bitfield);
                            drop(sent_have_bitfield);
                        }

                        TorrentTcpMessage::Choke => {
                            tracing::info!(
                                "Caught Choke from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            let _ = self.choked_sender.send(true);
                        }
                        TorrentTcpMessage::Unchoke => {
                            tracing::info!(
                                "Caught Unchoke from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            let _ = self.choked_sender.send(false);
                        }
                        TorrentTcpMessage::Have(piece_index) => {
                            tracing::info!(
                                "Caught Have from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            peer_bitfield.lock().await.set(piece_index);
                            let bitfield = shared_downloads.bitfield.read().await;
                            if !bitfield.has(piece_index) {
                                let _ = tasks_sender.send(Task::Interested).await;
                            }
                        }

                        TorrentTcpMessage::Piece {
                            index,
                            begin,
                            block,
                        } => {
                            tracing::info!(
                                "Caught Piece from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            let mut pieces = caught_piece_responses.lock().await;
                            let pr = PieceResponse {
                                index,
                                begin,
                                block,
                            };
                            // No duplicates stored
                            if !pieces.contains(&pr) {
                                pieces.push(pr);
                            } else {
                                tracing::error!(
                                    "Caught duplicate Piece from {}",
                                    String::from_utf8_lossy(&self.peer_id)
                                );
                            }
                        }
                        TorrentTcpMessage::Request {
                            index,
                            begin,
                            length,
                        } => {
                            tracing::info!(
                                "Caught Request from {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            if let Some(block) =
                                shared_downloads.get_block(index, begin, length).await
                            {
                                let response_task = Task::Response(PieceResponse {
                                    index,
                                    begin,
                                    block,
                                });
                                let _ = tasks_sender.send(response_task).await;
                            }
                        }                        
                        _ => {
                            return Err(Error::new(
                                ErrorKind::InvalidData,
                                "Unexpected TorrentTcpMessage received",
                            ));
                        }
                    }
                }
                Ok(())
            })
            .expect("Failed to create a Listener");

        let mut tasks_receiver = self.tasks_receiver;

        // Sender, for local requests only
        tokio::task::Builder::new()
            .name(&format!(
                "Sender for: {:?}",
                String::from_utf8_lossy(&self.peer_id)
            ))
            .spawn(async move {
            loop {
                if let Some(task) = tasks_receiver.recv().await {
                    match task {
                        Task::Interested => {
                            tracing::info!(
                                "Sending Interested {}",
                                String::from_utf8_lossy(&self.peer_id)
                            );
                            let _ =
                                Self::send_message(&mut sink, TorrentTcpMessage::Interested).await;
                        }
                        Task::Have(piece_index) => {
                            tracing::info!("Sending Have {}", piece_index);
                            let _ =
                                Self::send_message(&mut sink, TorrentTcpMessage::Have(piece_index))
                                    .await;
                        }
                        Task::Response(PieceResponse {
                            index,
                            begin,
                            block,
                        }) => {
                            tracing::info!(
                                "Sending Piece {}, index: {} ",
                                String::from_utf8_lossy(&self.peer_id),
                                index
                            );
                            let resonse_message = TorrentTcpMessage::Piece {
                                index,
                                begin,
                                block,
                            };
                            let _ = Self::send_message(&mut sink, resonse_message).await;
                        }
                        Task::Request(piece_req) => {
                            tracing::info!(
                                "Sending Request to {}, piece_index: {} ",
                                String::from_utf8_lossy(&self.peer_id),
                                piece_req.piece_index
                            );
                            let _ = Self::send_download_pieces(
                                &mut sink,
                                &mut choked_receiver,
                                &mut request_permits_receiver,
                                piece_req.clone(),
                            )
                            .await;
                            let _ = accepted_piece_req_sender.send(piece_req.clone()).await;
                        }
                    }
                }
            }
        }).expect("Failed to create a Sender");

        let peer_bitfield = self.peer_bitfield_arc.clone();
        let sent_have_bitfield = self.sent_have_bitfield_arc.clone();
        let shared_downloads = self.shared_downloads_arc.clone();
        let tasks_sender = self.tasks_sender.clone();

        // Tasks sender for the `Have` messages
        tokio::task::Builder::new()
            .name(&format!(
                "Haves sender for: {:?}",
                String::from_utf8_lossy(&self.peer_id)
            ))
            .spawn(async move {
                 loop {
                    let peer_bitfield = peer_bitfield.lock().await.clone();
                    let shared_downloads_bitfield = shared_downloads.bitfield.read().await.clone();
                    let mut sent_have_bitfield = sent_have_bitfield.lock().await;

                    let have_bitfield = shared_downloads_bitfield.diff(&peer_bitfield);
                    let have_unsent_bitfield = have_bitfield.diff(&sent_have_bitfield);

                    if !have_unsent_bitfield.is_empty() {
                        for piece_index in have_unsent_bitfield.get_set_indices() {
                            let _ = tasks_sender.send(Task::Have(piece_index)).await;
                            sent_have_bitfield.set(piece_index);
                        }
                    }
                drop(sent_have_bitfield);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }).expect("Failed to create a Haves sender");

        let caught_piece_responses = self.caught_piece_responses_arc.clone();
        let shared_downloads = self.shared_downloads_arc.clone();
        let peer_bitfield = self.peer_bitfield_arc.clone();

        // Bundler, creates DownloadedPiece's, break the connection if the peer misbehaves
        tokio::task::Builder::new()
            .name(&format!(
                "Bundler for: {:?}",
                String::from_utf8_lossy(&self.peer_id)
            ))
            .spawn(async move {
            while let Some(accepted_piece_req) = accepted_piece_req_receiver.recv().await {
                tracing::info!(
                    "Collecting blocks for the piece_index: {}",
                    accepted_piece_req.piece_index
                );
                let timer = tokio::time::Instant::now();
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                    let mut piece_responses_guard = caught_piece_responses.lock().await;
                    let mut piece_response: Vec<PieceResponse> = piece_responses_guard
                        .iter()
                        .filter(|bp| bp.index == accepted_piece_req.piece_index)
                        .cloned()
                        .collect();

                    if timer.elapsed() > tokio::time::Duration::from_secs(10) {
                        tracing::error!(
                            "Collecting took too long for the piece_index: {}",
                            accepted_piece_req.piece_index
                        );
                        peer_bitfield
                            .lock()
                            .await
                            .unset(accepted_piece_req.piece_index);
                        piece_responses_guard.retain(|pr| !piece_response.contains(&pr));

                        break;
                    }

                    let current_length: u32 =
                        piece_response.iter().map(|bp| bp.block.len() as u32).sum();

                    if current_length < accepted_piece_req.piece_length {
                        continue;
                    }
                    if current_length > accepted_piece_req.piece_length {
                        tracing::error!(
                            "Caught more pieces then expected! for piece_index: {}",
                            accepted_piece_req.piece_index
                        );
                        tracing::error!("Caught pieces trace:");
                        tracing::error!("{:?}", piece_response);

                        piece_responses_guard.retain(|pr| !piece_response.contains(&pr));
                        break;
                    }

                    piece_response.sort();

                    let piece_data: Vec<u8> = piece_response
                        .iter()
                        .flat_map(|piece| piece.block.iter())
                        .copied()
                        .collect();
                    let piece_hash: [u8; 20] = Sha1::digest(&piece_data).into();

                    if piece_hash == accepted_piece_req.piece_hash {
                        tracing::info!(
                            "Completed the piece_index: {}",
                            accepted_piece_req.piece_index
                        );

                        shared_downloads
                            .bitfield
                            .write()
                            .await
                            .set(accepted_piece_req.piece_index);
                        shared_downloads.pieces.write().await.push(PieceDownloaded {
                            piece_data,
                            piece_req: accepted_piece_req,
                        });
                        piece_responses_guard.retain(|pr| !piece_response.contains(&pr));
                        break;
                    } else {
                        tracing::error!(
                            "Wrong piece_hash received for the piece_index: {}",
                            accepted_piece_req.piece_index
                        );
                        tracing::error!("Caught pieces trace:");
                        tracing::error!("{:?}", piece_response);

                        return Err(Error::new(
                            ErrorKind::InvalidData,
                            "Wrong piece_hash received, break the connection",
                        ));
                    }
                }
            }
            return Ok(());
        }).expect("Failed to create a Builder");

        return Ok(());
    }
}

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
