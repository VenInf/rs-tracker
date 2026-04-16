use std::{io::{Error, ErrorKind}};
use sha1::{Digest, Sha1};
use std::fs::OpenOptions;
use std::io::{Write, Seek, SeekFrom};

use crate::{peer::{ConnectedPeer, TorrentTcpMessage}, torrent_file::{FileData, TorrentInfo}};

#[derive(Debug, Clone)]
pub struct Pieces {
    pub pieces_vec: Vec<Piece>,
    pub piece_length: u32,
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct Piece {
    pub piece_hash: [u8; 20],
    pub piece_data: Option<Vec<u8>>
}

#[derive(Debug, Clone)]
pub struct PieceTask {
    pub piece_hash: [u8; 20],
    pub piece_index: u32,
    pub piece_length: u32,
}

#[derive(Debug, Clone)]
pub struct PieceDownloaded {
    pub piece_data: Vec<u8>,
    pub piece_task: PieceTask
}

impl Pieces {
    pub fn new(info: &TorrentInfo) -> Result<Self, Error> {
        let FileData::Single { length } = info.file_data else {
            return Err(Error::new(ErrorKind::InvalidInput, "Multifile torrent is not supported"));
        };
        
        let mut pieces_vec = vec![]; 

        for hash in info.pieces.clone() {
            let piece = Piece { piece_hash: hash, piece_data: None};
            pieces_vec.push(piece);
        }
        
        Ok(Pieces { pieces_vec, piece_length: info.piece_length as u32, length: length as u32})
    }

    pub fn bitfield(self) -> Vec<u8> {
        self.pieces_vec.iter().map(|p| p.piece_data.is_some() as u8).collect()
    }

    pub fn write_to_disk(&self, filename: &str) -> Result<(), Error> {
        println!("Attempt to write to disc");

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(filename)?;

        file.set_len(self.length as u64)?;

        for (idx, piece) in self.pieces_vec.iter().enumerate() {
            let offset = (idx as u64) * (self.piece_length as u64);
            file.seek(SeekFrom::Start(offset))?;
            
            let Some(piece_data) = &piece.piece_data else { continue; };
            file.write_all(piece_data.as_slice())?;
        }

        Ok(())
    }    
}

