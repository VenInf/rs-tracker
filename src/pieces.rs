use std::{io::{Error, ErrorKind}};
use sha1::{Digest, Sha1};
use std::fs::OpenOptions;
use std::io::{Write, Seek, SeekFrom};
use tokio::sync::RwLock;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    Request(PieceReq),
    Response(PieceResponse),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PieceResponse {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PieceReq {
    pub piece_hash: [u8; 20],
    pub piece_index: u32,
    pub piece_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PieceDownloaded {
    pub piece_data: Vec<u8>,
    pub piece_req: PieceReq
}

pub struct SharedDownloads {
    pub bitfield: RwLock<Bitfield>,
    pub pieces: RwLock<Vec<PieceDownloaded>>,
}

impl SharedDownloads {
        pub async fn get_block(
        &self,
        index: u32,
        begin: u32,
        length: u32,
    ) -> Option<Vec<u8>> {
        let pieces_guard = self.pieces.read().await;
        let piece = pieces_guard.get(index as usize)?;

        let start = begin as usize;
        let end = (begin + length) as usize;

        if end <= piece.piece_data.len() {
            Some(piece.piece_data[start..end].to_vec())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitfield {
    pub bytes: Vec<u8>,
}

impl Bitfield {
    pub fn new(total_pieces_length: u64) -> Self {
        Self { bytes: vec![0; total_pieces_length.div_ceil(8) as usize] }
    }
}

impl Bitfield {
    pub fn has(&self, piece_index: u32) -> bool {
        let byte_index = piece_index / 8;
        let bit_offset = piece_index % 8;

        if let Some(byte) = self.bytes.get(byte_index as usize) {
            // BitTorrent bit order: index 0 is the most significant bit (128)
            let mask = 0b1000_0000 >> bit_offset;
            *byte == mask
        } else {
            false
        }
    }

    pub fn set(&mut self, piece_index: u32) {
        let byte_index = piece_index / 8;
        let bit_offset = piece_index % 8;

        if let Some(byte) = self.bytes.get_mut(byte_index as usize) {
            let mask = 0b1000_0000 >> bit_offset;
            *byte |= mask;
        }
    }

    pub fn set_all(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
    }
}

// TODO: remove when will be not needed
impl Pieces {
    pub fn new(info: &TorrentInfo) -> Result<Self, Error> {
        let FileData::Single { length } = info.file_data else {
            return Err(Error::new(ErrorKind::InvalidInput, "Multifile torrent is not supported"));
        };
        
        let mut pieces_vec = vec![]; 

        for piece_hash in info.piece_hashes.clone() {
            let piece = Piece { piece_hash, piece_data: None};
            pieces_vec.push(piece);
        }
        
        Ok(Pieces { pieces_vec, piece_length: info.piece_length as u32, length: length as u32})
    }

    pub fn bitfield(self) -> Vec<u8> {
        self.pieces_vec.iter().map(|p| p.piece_data.is_some() as u8).collect()
    }

}

