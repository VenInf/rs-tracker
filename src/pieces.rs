use std::{fmt, io::{Error, ErrorKind}};
use sha1::{Digest, Sha1};
use std::io::{Write, Seek, SeekFrom};
use tokio::sync::RwLock;
use crate::{peer::{ConnectedPeer, TorrentTcpMessage}, torrent_file::{FileData, TorrentInfo}};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Task {
    Interested,
    Have(u32),
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
    pub bytes: Vec<u8>, // Has to be equal to the amount of pieces
}

impl Bitfield {
    pub fn new(total_pieces_amount: u64) -> Self {
        Self { bytes: vec![0; total_pieces_amount.div_ceil(8) as usize] }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.iter().fold(0, |acc, x| acc | *x) == 0
    }

    pub fn is_full(&self) -> bool {
        self.bytes.iter().fold(1, |acc, x| acc & *x) == 1
    }

    pub fn is_close_to_done(&self) -> bool {
        let total_set = self.total_set();
        let total = 8 * self.bytes.len() as u32;
        total - total_set <= total.div_ceil(100) 
    }

    pub fn total_set(&self) -> u32 {
        self.bytes.iter()
            .map(|&byte| byte.count_ones() as u32)
            .sum()
    }

    pub fn has(&self, piece_index: u32) -> bool {
        let byte_index = piece_index / 8;
        let bit_offset = piece_index % 8;

        if let Some(byte) = self.bytes.get(byte_index as usize) {
            // BitTorrent bit order: index 0 is the most significant bit (128)
            let mask = 0b1000_0000 >> bit_offset;
            byte & mask != 0
        } else {
            false
        }
    }

    pub fn set(&mut self, piece_index: u32) {
        let byte_index = piece_index / 8;
        let bit_index = piece_index % 8;

        if let Some(byte) = self.bytes.get_mut(byte_index as usize) {
            let mask = 0b1000_0000 >> bit_index;
            *byte |= mask;
        }
    }

    pub fn set_all(&mut self, bytes: &Vec<u8>) {
        self.bytes = bytes.clone();
    }

    pub fn diff(&self, bitfield: &Bitfield) -> Bitfield {
        let bytes = self.bytes.iter()
                      .zip(bitfield.bytes.iter())
                      .map(|(b1, b2)| b1 & !b2 ) // Set in b1 but not set in b2
                      .collect();
        Bitfield { bytes }
    }

    pub fn get_set_indices(&self) -> Vec<u32> {
        self.bytes
            .iter()
            .enumerate()
            .flat_map(|(byte_index, &byte)| {
                if byte == 0 {
                    return (0..0).collect::<Vec<u32>>();
                }

                (0..8)
                    .filter(move |bit_index| {
                        let mask = 0b1000_0000 >> bit_index;
                        (byte & mask) != 0
                    })
                    .map(move |bit_index| byte_index as u32 * 8 + bit_index)
                    .collect::<Vec<u32>>()
            })
            .collect()
    }        
}

impl std::fmt::Display for Bitfield {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Take up to 4 bytes (32 pieces)
        let display_limit = 4.min(self.bytes.len());
        let mut bits = String::new();

        for i in 0..display_limit {
            bits.push_str(&format!("{:08b}", self.bytes[i]));
            if i < display_limit - 1 { bits.push(' '); }
        }

        if self.bytes.len() > 4 {
            bits.push_str("...");
        }

        write!(f, "[{}]", bits)
    }
}
