use crate::bencoding_parser as BP;
use crate::torrent_file as TF;
use std::net::Ipv4Addr;

#[derive(Debug)]
pub struct AnnounceResponse {
    pub interval: i64,
    pub peers: Vec<(String, u16)>,
}


pub fn parse_announce_response<'a>(ast: BP::AST<'a>) -> Result<AnnounceResponse, TF::ConversionError> {
    let interval = ast.get_int(b"interval").ok_or(TF::ConversionError::new("interval"))?;
    
    let Some(BP::AST::ByteString(peers_raw)) = ast.get_from_dict(b"peers") else { return Err(TF::ConversionError::new("peers")) };
    let peers = peers_raw.chunks_exact(6)
            .map(|chunk| {
            let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
            
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            
            (ip.to_string(), port)
        })
        .collect();

    Ok(AnnounceResponse { interval, peers })
}