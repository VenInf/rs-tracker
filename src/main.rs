mod bencoding_parser;
mod torrent_file;

use std::fs::File;
use std::io;
use std::io::Read;
use std::path::PathBuf;

#[derive(clap::Parser)]
#[command(author, version, about = "File reader")]
struct Cli {
    path: PathBuf,
}

// TODO: add parser that parses all utf-8 characters with `winnow`

fn main() -> io::Result<()> {
    let args = <Cli as clap::Parser>::parse();
    let file_path = &args.path;
    
    let mut f = File::open(file_path)?;
    let mut input_vec: Vec<u8> = Vec::new();

    f.read_to_end(&mut input_vec)?;

    let mut input_slice = input_vec.as_slice();
    

    println!("Read {} bytes from {}", input_slice.len(), file_path.display());

    let output = bencoding_parser::parse_bencode(&mut input_slice).unwrap();

    println!("Parse output is");
    println!("{}", output);

    let torrent = torrent_file::bentree_to_torrent_file(&output);

    // println!("Torrent file object is");
    // println!("{:?}", torrent);

    return Ok(());
}
