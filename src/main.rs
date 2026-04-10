mod bencoding_parser;
mod torrent_file;
mod announce;
mod announce_response;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

#[derive(clap::Parser)]
#[command(author, version, about = "torrent tracker")]
struct Cli {
    path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = <Cli as clap::Parser>::parse();
    let file_path = &args.path;
    
    let mut f = File::open(file_path)?;
    let mut input_vec: Vec<u8> = Vec::new();

    f.read_to_end(&mut input_vec)?;

    let mut input_slice = input_vec.as_slice();

    let torrent_ast = bencoding_parser::parse_bencode(&mut input_slice).unwrap();
    println!("Torrent AST: {}", torrent_ast);

    let torrent_file = torrent_file::bentree_to_torrent_file(&torrent_ast)?;
    println!("Torrent File: {:?}", torrent_file);

    // let Some(ref announce_urls) = torrent_file.announce_list else {
    //     return Err("No announce_urls".into());
    // };

    let Some(announce_url) = torrent_file.announce else {
        return Err("No announce_url".into());
    };
    
    let peer_id: &[u8; 20] = b"-GT0001-os9964142397"; // TODO: make a proper peer-id generator
    let announce_response = announce::announce_to_tracker(announce_url, peer_id, &torrent_file, 6881).await?;
    
    println!("Received {} bytes from tracker.", announce_response.len());
    let announce_bytes: &mut &[u8] = &mut announce_response.as_slice();
    let save_path = format!("./samples/{}.bencode", torrent_file.comment.unwrap_or("no-name"));
    let _ = announce::save_file(&save_path, announce_bytes).await;
    let announce_ast = bencoding_parser::parse_bencode(announce_bytes).unwrap();

    println!("{}", announce_ast);

    let announce_response = announce_response::parse_announce_response(announce_ast);
    println!("{:?}", announce_response);

    return Ok(());
}
