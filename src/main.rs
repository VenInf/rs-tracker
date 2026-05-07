mod announce;
mod bencoding_parser;
mod handshake;
mod peer;
mod pieces;
mod torrent_file;
mod torrent_tcp_message;

use crate::pieces::{Bitfield, PieceDownloaded, PieceRequest, SharedDownloads, Task};
use crate::torrent_file::TorrentFile;
use crate::torrent_file::FileData;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Error, ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Duration, timeout};

#[derive(clap::Parser)]
#[command(author, version, about = "torrent client")]
struct Cli {
    path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    console_subscriber::init();

    let args = <Cli as clap::Parser>::parse();
    let file_path = &args.path;

    let mut f = File::open(file_path)?;
    let mut input_vec: Vec<u8> = Vec::new();

    f.read_to_end(&mut input_vec)?;

    let mut input_slice = input_vec.as_slice();

    let torrent_ast = bencoding_parser::parse_bencode(&mut input_slice).unwrap();
    println!("Torrent AST: {}", torrent_ast);

    let torrent_file = torrent_file::bentree_to_torrent_file(&torrent_ast).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "Failed to get torrent file from ast",
        )
    })?;

    let Some(announce_url) = &torrent_file.announce else {
        return Err(Error::new(ErrorKind::InvalidData, "No announce-url"));
    };

    let my_peer_id: &[u8; 20] = b"-GT0001-os9964142111"; // TODO: make a proper peer-id generator
    // TODO: make a thread that serves port 6881
    let announce_response =
        announce::announce_to_tracker(announce_url, my_peer_id, &torrent_file, 6881).await?;

    let announce_bytes: &mut &[u8] = &mut announce_response.as_slice();
    let announce_ast = bencoding_parser::parse_bencode(announce_bytes).unwrap();

    let announce_response = announce::parse_announce_response(announce_ast)
        .map_err(|_| Error::new(ErrorKind::InvalidData, "Failed to parse announce response"))?;
    println!("{:?}", announce_response);

    let total_amount_of_pieces = torrent_file.info.piece_hashes.len() as u32;
    println!("total_amount_of_pieces: {}", total_amount_of_pieces);

    let piece_length = torrent_file.info.piece_length as u32;
    println!("piece_length: {}", piece_length);

    let remainder = torrent_file.info.file_data.clone().total_length() as u32 % piece_length;
    let last_piece_length = if remainder == 0 {
        piece_length
    } else {
        remainder
    };

    println!("last_piece_length: {}", last_piece_length);

    let all_requests: Vec<PieceRequest> = torrent_file
        .info
        .piece_hashes
        .iter()
        .enumerate()
        .map(|(piece_index, piece_hash)| {
            if (piece_index as u32) == total_amount_of_pieces - 1 {
                PieceRequest {
                    piece_hash: piece_hash.clone(),
                    piece_index: (piece_index as u32),
                    piece_length: last_piece_length,
                }
            } else {
                PieceRequest {
                    piece_hash: piece_hash.clone(),
                    piece_index: (piece_index as u32),
                    piece_length: piece_length,
                }
            }
        })
        .collect();

    let piece_requests_arc: Arc<Mutex<Vec<PieceRequest>>> = Arc::new(Mutex::new(all_requests));

    let peers = announce_response.peers.clone();
    let task_channels: Vec<(mpsc::Sender<Task>, mpsc::Receiver<Task>)> =
        (0..peers.len()).map(|_| mpsc::channel(16)).collect();

    let shared_downloads_arc = Arc::new(SharedDownloads {
        bitfield: RwLock::new(Bitfield::new(total_amount_of_pieces)),
        pieces: RwLock::new(vec![]),
    });

    let mut peer_init_set = JoinSet::new();

    for (peer_address, (task_sender, task_receiver)) in peers.into_iter().zip(task_channels) {
        let shared_downloads = shared_downloads_arc.clone();

        peer_init_set
            .build_task()
            .name(&format!("Initializing peer: {}", peer_address.0))
            .spawn(async move {
                let peer_part = peer::ConnectedPeer::new(
                    peer_address.clone(),
                    torrent_file.info_hash,
                    my_peer_id.clone(),
                    total_amount_of_pieces,
                    task_sender.clone(),
                    task_receiver,
                    shared_downloads,
                );
                let peer_res = timeout(Duration::from_secs(10), peer_part)
                    .await
                    .map_err(|_| Error::new(ErrorKind::TimedOut, "Deadline has elapsed"))
                    .flatten();
                (task_sender, peer_res)
            })
            .expect("Failed to spawn");
    }

    let mut connected_peers = vec![];
    let mut task_senders = vec![];

    while let Some(set_res) = peer_init_set.join_next().await {
        match set_res {
            Ok((task_sender, Ok(connected_peer))) => {
                connected_peers.push(connected_peer);
                task_senders.push(task_sender);
            }
            Ok((_, Err(e))) => {
                tracing::error!("Connection failed with {}", e);
            }
            Err(e) => {
                tracing::error!("Connection failed with {}", e);
            }
        }
    }

    let peer_bitfields: Vec<Arc<Mutex<Bitfield>>> = connected_peers
        .iter()
        .map(|peer| peer.peer_bitfield_arc.clone())
        .collect();

    // Peer threads
    for peer in connected_peers {
        tokio::task::Builder::new()
            .name(&format!(
                "Call interact_loop on peer with id: {}",
                &peer.peer_id
            ))
            .spawn(async move {
                let peer_result = peer.interact_loop().await;

                if let Err(e) = peer_result {
                    tracing::error!("Peer connection lost: {}", e);
                }
            })
            .expect("Failed to call peer.interact_loop()");
    }

    let shared_downloads = shared_downloads_arc.clone();

    // Thread that sends out the tasks to the peers
    tokio::task::Builder::new()
        .name(&format!("Main request sender thread"))
        .spawn(async move {
            loop {
                let mut current_piece_requests = piece_requests_arc.lock().await.clone();
                current_piece_requests.reverse();

                // Send out requests
                let mut peer_index = 0;
                while let Some(piece_request) = current_piece_requests.pop() {
                    if peer_bitfields[peer_index]
                        .lock()
                        .await
                        .has(piece_request.piece_index)
                    {
                        if let Err(_) = task_senders[peer_index]
                            .send_timeout(
                                Task::Request(piece_request.clone()),
                                Duration::from_millis(100),
                            )
                            .await
                        {
                            tracing::info!(
                                "Failed to push piece_request.piece_index {} to peer_index {}",
                                piece_request.piece_index,
                                peer_index
                            );
                            peer_index = (peer_index + 1) % peer_bitfields.len();
                        } else {
                            tracing::info!(
                                "Pushed piece_request.piece_index {} to peer_index {}",
                                piece_request.piece_index,
                                peer_index
                            );
                        }
                    } else {
                        peer_index = (peer_index + 1) % peer_bitfields.len();
                    }
                }

                tracing::info!("Sent all pending requests");

                // Remove downloaded requests
                let current_bitfield = shared_downloads.bitfield.read().await.clone();
                let mut piece_requests_guard = piece_requests_arc.lock().await;
                piece_requests_guard.retain(|req| !current_bitfield.has(req.piece_index));
                drop(piece_requests_guard);

                tracing::info!("Removed all downloaded requests, waiting for another cycle");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        })
        .expect("Failed to create the main sender");

    // Thread that writes to disk once there is enough data
    let shared_downloads = shared_downloads_arc.clone();
    loop {
        let bitfield = shared_downloads.bitfield.read().await.clone();
        tracing::info!("Total pieces to download: {}", bitfield.total());
        tracing::info!("Total downloaded pieces: {}", bitfield.total_set());

        if bitfield.is_full() {
            let downloaded_pieces = shared_downloads.pieces.read().await.clone();
            write_to_disk(downloaded_pieces, torrent_file.clone())?;
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}

pub fn write_to_disk(
    pieces_downloaded: Vec<PieceDownloaded>,
    torrent_file: TorrentFile,
) -> Result<(), Error> {
    println!("Attempt to write to disc");
    
    let total_downloaded_length: u64 = pieces_downloaded
        .iter()
        .map(|p| p.piece_req.piece_length as u64)
        .sum();

    let downloaded_data: Vec<u8> = {
        let mut pieces = pieces_downloaded.clone();
        pieces.sort_by(|p1, p2| p1.piece_req.piece_index.cmp(&p2.piece_req.piece_index));
        pieces.into_iter().flat_map(|p| p.piece_data).collect()
    };

    match torrent_file.info.file_data {
        FileData::Single { length } => {
            assert_eq!(total_downloaded_length, length);
            let filename = torrent_file.info.name;
            let mut file = OpenOptions::new().write(true).create(true).open(filename)?;
            file.set_len(total_downloaded_length)?;
            file.write_all(downloaded_data.as_slice())?;
        },
        FileData::Multi { files } => {
            let mut file_offset_start: usize = 0;
            for file_info in files {
                let filename = file_info.path.concat();
                let mut file = OpenOptions::new().write(true).create(true).open(filename)?;
                file.set_len(file_info.length)?;

                let file_offset_end = file_offset_start + file_info.length as usize;
                let data_slice = &downloaded_data[file_offset_start..file_offset_end];
                file.write_all(data_slice)?;

                file_offset_start = file_offset_end;            
            }
        }
    }
    Ok(())

}
