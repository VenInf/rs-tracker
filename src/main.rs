mod bencoding_parser;
mod torrent_file;
mod announce;

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
    let torrent_file = torrent_file::bentree_to_torrent_file(&torrent_ast)?;

    println!("Torrent AST: {}", torrent_ast);
    println!("Torrent File: {:?}", torrent_file);

    // let Some(ref announce_urls) = torrent_file.announce_list else {
    //     return Err("No announce_urls".into());
    // };

    let Some(announce_url) = torrent_file.announce else {
        return Err("No announce_url".into());
    };
    
    let peer_id: &[u8; 20] = b"-GT0001-os9964142397"; // TODO: make a proper peer-id generator
    let announce_response = announce::announce_to_tracker(announce_url, peer_id, &torrent_file, 6881);
    
    match announce_response.await {
        Ok(bytes) => {
            println!("Received {} bytes from tracker.", bytes.len());
            let bytes_array: &mut &[u8] = &mut bytes.as_slice();
            let _ = announce::save_file("./samples/Wet_Fly_Fishing_Announce.bencode", bytes_array).await;
            let ast = bencoding_parser::parse_bencode(bytes_array).unwrap();
            println!("{}", ast);

        }
        Err(e) => {
            eprintln!("Tracker announcement failed: {}", e);
        }
    }

    
    
    return Ok(());
}
