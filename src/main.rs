mod parser;

use std::fmt::format;
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
    // println!("Parse input is {:?}", input_slice);

    let output = parser::parse_bencode(&mut input_slice).unwrap();

    println!("Parse output is");
    println!("{}", output);

    return Ok(());
}
