use reqwest::Error;
use crate::peer::ConnectedPeer;
// TODO: cnahge the size to the box type

#[derive(Debug, Clone)]
pub struct Pieces {
    pub pieces: Vec<Piece>,
    pub piece_length: i64,
    pub length: i64,
}

#[derive(Debug, Clone)]
pub struct Piece {
    pub piece_hash: [u8; 20],
    pub piece_data: Option<Vec<u8>>
}

impl Pieces {
    pub fn bitfield(self) -> Vec<u8> {
        self.pieces.iter().map(|p| p.piece_data.is_some() as u8).collect()
    }


    async fn download_from(self, peer: &mut ConnectedPeer, piece_length: i64, piece_index: u32) -> Result<Vec<u8>, Error> {
        let block_size = 16384;
        let mut buffer = vec![0u8; piece_length as usize];

        for offset in (0..piece_length).step_by(block_size as usize) {
            let len = std::cmp::min(block_size, piece_length - offset);
    
            let response = peer.send_request(piece_index, offset, len).await?;
            
            // TODO: add the block to the buffer

        }

        Ok(buffer)
    }
}

