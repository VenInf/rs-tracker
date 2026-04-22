use crate::bencoding_parser as BP;
use crate::torrent_file as TF;
use std::io::{Error, ErrorKind};
use std::net::Ipv4Addr;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use url::form_urlencoded;

pub async fn announce_to_tracker<'a>(
    url: &String,
    peer_id: &[u8; 20],
    torrent_file: &TF::TorrentFile,
    port: u16,
) -> Result<Vec<u8>, Error> {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("peer_id", unsafe { std::str::from_utf8_unchecked(peer_id) })
        .append_pair("info_hash", unsafe {
            std::str::from_utf8_unchecked(&torrent_file.info_hash)
        })
        .append_pair("port", &port.to_string())
        .append_pair("uploaded", "0") // TODO: dynamic the uploaded/downloaded numbers
        .append_pair("downloaded", "0")
        .append_pair("left", &torrent_file.left_initial.to_string())
        .append_pair("compact", "1")
        .finish();

    let full_url = format!("{}?{}", url, query);

    println!("Full url is: {}", full_url);

    let client = reqwest::Client::new();

    let response = client.get(&full_url).send().await.map_err(|_e| {
        Error::new(
            ErrorKind::ConnectionRefused,
            format!("Failed to get {}", full_url),
        )
    })?;

    let bytes = response
        .bytes()
        .await
        .map_err(|_e| Error::new(ErrorKind::InvalidData, "Failed to get bytes"))?;

    Ok(bytes.to_vec())
}

pub async fn save_file(path: &str, data: &[u8]) -> Result<(), Error> {
    let mut file = File::create(path).await?;
    file.write_all(data).await?;
    Ok(())
}

#[derive(Debug)]
pub struct AnnounceResponse {
    pub interval: i64,
    pub peers: Vec<(String, u16)>,
}

pub fn parse_announce_response<'a>(
    ast: BP::AST<'a>,
) -> Result<AnnounceResponse, TF::ConversionError> {
    let interval = ast
        .get_int(b"interval")
        .ok_or(TF::ConversionError::new("interval"))?;

    let Some(BP::AST::ByteString(peers_raw)) = ast.get_from_dict(b"peers") else {
        return Err(TF::ConversionError::new("peers"));
    };
    let peers = peers_raw
        .chunks_exact(6)
        .map(|chunk| {
            let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);

            let port = u16::from_be_bytes([chunk[4], chunk[5]]);

            (ip.to_string(), port)
        })
        .collect();

    Ok(AnnounceResponse { interval, peers })
}
