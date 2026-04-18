mod announce;
mod bencoding_parser;
mod torrent_file;
mod handshake;
mod peer;
mod pieces;

use std::sync::Arc;
use std::time::Duration;
use std::fs::File;
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tokio::task::JoinSet;
use tokio::time::timeout;
use rand::seq::IndexedRandom;
use tokio::sync::{Mutex, mpsc};
use crate::peer::TorrentTcpMessage;
use crate::pieces::{Bitfield, PieceDownloaded, PieceReq, PieceResponse, SharedDownloads, Task};
use crate::torrent_file::FileData;
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
    println!("Torrent File: {:?}", &torrent_file);

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


    let total_pieces_length = torrent_file.info.piece_length as u32;
    let piece_length = torrent_file.info.piece_length as u32;
    let remainder = torrent_file.info.piece_hashes.len() as u32 % piece_length;
    let last_piece_length = if remainder == 0 {
        piece_length
    } else {
        remainder
    };

    let all_requests: Vec<PieceReq> = torrent_file.info.piece_hashes.iter()
                                                      .enumerate()
                                                      .map(|(piece_index, piece_hash)|
                                                        if (piece_index as u32) == total_pieces_length - 1 {
                                                            PieceReq {piece_hash: piece_hash.clone(), piece_index: (piece_index as u32), piece_length: last_piece_length}
                                                        } else {
                                                            PieceReq {piece_hash: piece_hash.clone(), piece_index: (piece_index as u32), piece_length: piece_length}
                                                        })
                                                       .collect();

    let piece_requests: Arc<Mutex<Vec<PieceReq>>> = Arc::new(Mutex::new(all_requests));
    let downloaded_arc: Arc<Mutex<Vec<PieceDownloaded>>> = Arc::new(Mutex::new(vec![]));

    let total_pieces_length = torrent_file.info.file_data.total_length();

    let task_channels: Vec<(mpsc::Sender<Task>, mpsc::Receiver<Task>)> = (0..announce_response.peers.len())
        .map(|_| mpsc::channel(64))
        .collect();

    let (downloaded_sender, mut downloaded_receiver) = mpsc::channel(total_pieces_length as usize);

    let mut connected_peers = vec![];
    let mut task_senders = vec![];

    let shared_downloads = Arc::new(SharedDownloads {
        bitfield: RwLock::new(Bitfield::new(total_pieces_length)),
        pieces: RwLock::new(vec![]),
    });

    for (peer_address, (task_sender, task_receiver)) in announce_response.peers.iter().zip(task_channels) {
        let peer = peer::ConnectedPeer::new(
                    peer_address.clone(),
                    torrent_file.info_hash,
                    my_peer_id.clone(),
                    total_pieces_length,
                    task_sender.clone(),
                    task_receiver,
                    downloaded_sender.clone(),
                    shared_downloads.clone()
                    ).await;
        
        match peer {
            Ok(connected_peer) => {
                connected_peers.push(connected_peer);
                task_senders.push(task_sender);

            }
            Err(e) => { println!("Connection to {:?} peer failed with {}", peer_address, e); }
        }
    }

    let peer_bitfields: Vec<Arc<Mutex<Bitfield>>> = connected_peers.iter().map(|peer| peer.peer_bitfield.clone()).collect(); 

    // Peer threads
    for peer in connected_peers {        
        tokio::spawn(async move {
            println!("Call interact_loop on peer with id: {:?}", peer.peer_id);
            if let Err(e) = peer.interact_loop().await {
                eprintln!("Peer connection lost: {}", e);
            }
        });
    }

    // Thread that sends out the tasks to the peers
    let downloaded = downloaded_arc.clone();
    tokio::spawn(async move {
        // Remove downloaded requests
        let downloaded = downloaded.lock().await.clone();
        piece_requests.lock().await.retain(|req| !downloaded.iter().any(|d| &d.piece_req == req));

        // Send out requests
        for piece_req in piece_requests.lock().await.iter() {
            for (peer_index, peer_bitfield) in peer_bitfields.iter().enumerate() {
                let bf = peer_bitfield.lock().await.clone();
                if bf.has(piece_req.piece_index) {
                    let _ = task_senders[peer_index].send(Task::Request(piece_req.clone())).await;
                };
            }
        }

        if downloaded.len() as u64 == total_pieces_length {
            // TODO: write to a file
            let file_name = torrent_file.info.name.clone();
            write_to_disk(downloaded, file_name);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
    });

    let downloaded = downloaded_arc.clone();
    // Thread that collects the downloads and removes them from the tasks
    tokio::spawn(async move {
       while let Some(d) = downloaded_receiver.recv().await {
            let mut shared_downloads_write = shared_downloads.bitfield.write().await;
            shared_downloads_write.set(d.piece_req.piece_index);
            // TODO: send the have messages

            downloaded.lock().await.push(d);
       }
    });

    Ok(())
}
