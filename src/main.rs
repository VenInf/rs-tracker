mod announce;
mod bencoding_parser;
mod torrent_file;
mod handshake;
mod peer;
mod pieces;

use std::time::Duration;
use std::fs::File;
use std::io::{Error, ErrorKind, Read};
use std::path::PathBuf;
use tokio::time::timeout;

use crate::peer::TorrentTcpMessage;

#[derive(clap::Parser)]
#[command(author, version, about = "torrent tracker")]
struct Cli {
    path: PathBuf,
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
    println!("Torrent File: {:?}", torrent_file);

    let Some(announce_url) = torrent_file.announce else {
        return Err(Error::new(ErrorKind::InvalidData, "No announce-url"));
    };

    let my_peer_id: &[u8; 20] = b"-GT0001-os9964142397"; // TODO: make a proper peer-id generator
    let announce_response =
        announce::announce_to_tracker(announce_url, my_peer_id, &torrent_file, 6881).await?;

    println!("Received {} bytes from tracker.", announce_response.len());
    let announce_bytes: &mut &[u8] = &mut announce_response.as_slice();
    let save_path = format!(
        "./samples/{}.bencode",
        torrent_file.comment.unwrap_or("no-name")
    );
    let _ = announce::save_file(&save_path, announce_bytes).await;
    let announce_ast = bencoding_parser::parse_bencode(announce_bytes).unwrap();

    println!("{}", announce_ast);

    let announce_response = announce::parse_announce_response(announce_ast).map_err(|_| Error::new(ErrorKind::InvalidData, "Failed to parse announce response"))?;
    println!("{:?}", announce_response);
    

    let peer_address = announce_response.peers.first().ok_or(Error::new(ErrorKind::InvalidData, "No first peer to select"))?;
    let mut connected_peer = peer::ConnectedPeer::new(peer_address.clone(), torrent_file.info_hash, my_peer_id.clone()).await?;
    
    let mut bitfield = vec![];
    let mut chocked = true;

    println!("DEBUG: Waiting for a bitfield");
    let init_response = timeout(Duration::from_secs(10), connected_peer.read_message()).await?.ok();
    if let Some(TorrentTcpMessage::Bitfield(bf)) = init_response {
        println!("DEBUG: Caught bitfield: {:?}", bf);
        bitfield = bf;
    } else if let Some(TorrentTcpMessage::Unchoke) = init_response {
        println!("DEBUG: Caught unchocke message");
        chocked = false;
    } else {
        println!("DEBUG: No intial message sent");
    }

    let interested_message = TorrentTcpMessage::Interested;            
    connected_peer.send_message(interested_message).await?;

    let unchoke_response = timeout(Duration::from_secs(10), connected_peer.read_message()).await?.ok();
    if let Some(TorrentTcpMessage::Unchoke) = unchoke_response {
        println!("DEBUG: Caught unchocke message");
        chocked = false;
    } else {
        println!("DEBUG: No response on interested message sent");
    }

    if chocked {
        return Err(Error::new(ErrorKind::ConnectionRefused, format!("Failed to unchoke")));
    }

    let mut pieces = pieces::Pieces::new(&torrent_file.info)?;
    
    for (idx, piece) in pieces.pieces_vec.iter_mut().enumerate() {
        piece.download_from(&mut connected_peer, pieces.piece_length, idx as u32).await?
    }    

    pieces.write_to_disk(torrent_file.comment.unwrap_or("no-name"))?;

    return Ok(());
}
