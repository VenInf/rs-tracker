use std::error::Error;
use url::form_urlencoded;
use tokio::io::AsyncWriteExt;
use tokio::fs::File;
use crate::torrent_file as TF;

pub async fn announce_to_tracker<'a>(
    url: &str,
    peer_id: &[u8; 20],
    torrent_file: &TF::TorrentFile<'a>,
    port: u16,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("peer_id", unsafe { std::str::from_utf8_unchecked(peer_id) })
        .append_pair("info_hash", unsafe { std::str::from_utf8_unchecked(&torrent_file.info_hash) })
        .append_pair("port", &port.to_string())
        .append_pair("uploaded", "0")
        .append_pair("downloaded", "0")
        .append_pair("left", &torrent_file.left_initial.to_string())
        .append_pair("compact", "1")
        .finish();

    let full_url = format!("{}?{}", url, query);

    println!("Full url is: {}", full_url);

    let client = reqwest::Client::new();
    
    let response = client.get(full_url).send().await?;
    
    let bytes = response.bytes().await?;

    Ok(bytes.to_vec())
}

pub async fn save_file(path: &str, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::create(path).await?;
    file.write_all(data).await?;
    Ok(())
}