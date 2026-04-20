mod announce;
mod bencoding_parser;
mod torrent_file;
mod handshake;
mod peer;
mod pieces;

use std::sync::Arc;
use std::thread::current;
use std::time::Duration;
use std::fs::File;
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tokio::time::timeout;
use rand::seq::IndexedRandom;
use tokio::sync::{Mutex, mpsc};
use crate::pieces::{Bitfield, PieceDownloaded, PieceReq, SharedDownloads, Task};
use tokio::sync::RwLock;
use std::fs::OpenOptions;


#[derive(clap::Parser)]
#[command(author, version, about = "torrent tracker")]
struct Cli {
    path: PathBuf,
}

pub fn write_to_disk(pieces_downloaded: Vec<PieceDownloaded>, filename: String) -> Result<(), Error> {
    println!("Attempt to write to disc");

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(filename)?;

    let length: u64 = pieces_downloaded.iter().map(|p| p.piece_req.piece_length as u64).sum();

    file.set_len(length)?;

    for (idx, piece) in pieces_downloaded.iter().enumerate() {
        let offset = (idx as u64) * (piece.piece_req.piece_length as u64);
        file.seek(SeekFrom::Start(offset))?;
        
        file.write_all(piece.piece_data.as_slice())?;
    }
    Ok(())
}    


#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = <Cli as clap::Parser>::parse();
    let file_path = &args.path;

    let mut f = File::open(file_path)?;
    let mut input_vec: Vec<u8> = Vec::new();

    f.read_to_end(&mut input_vec)?;

    let mut input_slice = input_vec.as_slice();

    let torrent_ast = bencoding_parser::parse_bencode(&mut input_slice).unwrap();
    println!("Torrent AST: {}", torrent_ast);

    let torrent_file = torrent_file::bentree_to_torrent_file(&torrent_ast).map_err(|_| Error::new(ErrorKind::InvalidInput, "Failed to get torrent file from ast"))?;

    let Some(announce_url) = &torrent_file.announce else {
        return Err(Error::new(ErrorKind::InvalidData, "No announce-url"));
    };

    let my_peer_id: &[u8; 20] = b"-GT0001-os9964142397"; // TODO: make a proper peer-id generator
    let announce_response =
        announce::announce_to_tracker(announce_url, my_peer_id, &torrent_file, 6881).await?;

    println!("Received {} bytes from tracker.", announce_response.len());
    let announce_bytes: &mut &[u8] = &mut announce_response.as_slice();
    let save_path = format!(
        "./samples/{}.bencode",
        torrent_file.comment.unwrap_or(String::from("no-name"))
    );
    let _ = announce::save_file(&save_path, announce_bytes).await;
    let announce_ast = bencoding_parser::parse_bencode(announce_bytes).unwrap();

    println!("{}", announce_ast);

    let announce_response = announce::parse_announce_response(announce_ast).map_err(|_| Error::new(ErrorKind::InvalidData, "Failed to parse announce response"))?;
    println!("{:?}", announce_response);

    let total_amount_of_pieces = torrent_file.info.piece_hashes.len() as u64;
    let piece_length = torrent_file.info.piece_length as u32;
    let remainder = torrent_file.info.piece_hashes.len() as u32 % piece_length;
    let last_piece_length = if remainder == 0 {
        piece_length
    } else {
        remainder
    };

    let all_requests: Vec<PieceReq> = torrent_file.info
                                                  .piece_hashes.iter()
                                                  .enumerate()
                                                  .map(|(piece_index, piece_hash)|
                                                   if (piece_index as u64) == total_amount_of_pieces - 1 {
                                                      PieceReq {piece_hash: piece_hash.clone(), piece_index: (piece_index as u32), piece_length: last_piece_length}
                                                   } else {
                                                      PieceReq {piece_hash: piece_hash.clone(), piece_index: (piece_index as u32), piece_length: piece_length}
                                                   })
                                                   .collect();

    let piece_requests_arc: Arc<Mutex<Vec<PieceReq>>> = Arc::new(Mutex::new(all_requests));

    let peers = &announce_response.peers.clone();
    let task_channels: Vec<(mpsc::Sender<Task>, mpsc::Receiver<Task>)> = (0..peers.len())
        .map(|_| mpsc::channel(4))
        .collect();

    let shared_downloads_arc = Arc::new(SharedDownloads {
        bitfield: RwLock::new(Bitfield::new(total_amount_of_pieces)),
        pieces: RwLock::new(vec![]),
    }); // Add a way to send the have messages to the tasks

    let mut connected_peers = vec![];
    let mut task_senders = vec![];

    for (peer_address, (task_sender, task_receiver)) in peers.iter().zip(task_channels) {
        let peer_part = peer::ConnectedPeer::new(
                    peer_address.clone(),
                    torrent_file.info_hash,
                    my_peer_id.clone(),
                    total_amount_of_pieces,
                    task_sender.clone(),
                    task_receiver,
                    shared_downloads_arc.clone()
                    );
        let peer = timeout(Duration::from_secs(1), peer_part).await;

        match peer {
            Ok(Ok(connected_peer)) => {
                connected_peers.push(connected_peer);
                task_senders.push(task_sender);

            }
            Ok(Err(e)) => { println!("Connection to {} peer failed with {}", peer_address.0, e); }
            Err(e) => { println!("Connection to {} peer failed with {}", peer_address.0, e); }
        }
    }

    let peer_bitfields: Vec<Arc<Mutex<Bitfield>>> = connected_peers.iter().map(|peer| peer.peer_bitfield.clone()).collect(); 

    // Peer threads
    for peer in connected_peers {        
        tokio::spawn(async move {
            println!("Call interact_loop on peer with id: {:?}", String::from_utf8_lossy(&peer.peer_id));
            let peer_result = peer.interact_loop().await;

            if let Err(e) = peer_result {
                eprintln!("Peer connection lost: {}", e);
            }
        });
    }
    
    // Thread that sends out the tasks to the peers
    let shared_downloads = shared_downloads_arc.clone();
    tokio::spawn(async move {
        loop {
            println!("In the task sending thread");
                    
            // Remove downloaded requests
            let current_bitfield = shared_downloads.bitfield.read().await.clone();
            println!("current_bitfield.total_set() {}", current_bitfield.total_set());

            let mut piece_requests_guard = piece_requests_arc.lock().await;
            piece_requests_guard.retain(|req| !current_bitfield.has(req.piece_index));
            let current_piece_requests = piece_requests_guard.clone();
            drop(piece_requests_guard);

            // Send out requests
            for piece_req in current_piece_requests.iter() {
                for (peer_index, peer_bitfield) in peer_bitfields.iter().enumerate() {
                    if peer_bitfield.lock().await.is_empty() {
                        continue;
                    }

                    let has_piece = {
                        let bf = peer_bitfield.lock().await;
                        bf.has(piece_req.piece_index)
                    };                    

                    if has_piece {
                        let send_res = task_senders[peer_index]
                                .send_timeout(Task::Request(piece_req.clone()), Duration::from_millis(50)) // TODO: better solution?
                                .await;

                        if let Ok(()) = send_res {
                            println!("Sent request for piece_index: {} to peer_index: {}", piece_req.piece_index, peer_index);
                            break;
                        }
                    };
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        }
    });

    // Thread that writes to disk once there is enough data
    let shared_downloads = shared_downloads_arc.clone();
    loop {
        let bitfield = shared_downloads.bitfield.read().await.clone();
        println!("Total downloaded pieces: {}", bitfield.total_set());

        if bitfield.is_full() {
            let file_name = torrent_file.info.name.clone();
            let downloaded_pieces = shared_downloads.pieces.read().await.clone();
            let _ = write_to_disk(downloaded_pieces, file_name);
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
