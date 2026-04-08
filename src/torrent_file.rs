use crate::bencoding_parser as BP;
use std::{error::Error, fmt};

#[derive(Debug)]
pub struct TorrentFile<'a> {
    pub announce: &'a str,
    pub announce_list: Vec<Vec<&'a str>>,
    pub comment: &'a str,
    pub created_by: &'a str,
    pub creation_date: i64,
    pub encoding: &'a str,
    pub info: TorrentInfo<'a>,
    pub publisher: &'a str,
    pub publisher_url: &'a str,
}

#[derive(Debug)]
pub struct TorrentInfo<'a> {
    pub name: &'a str,
    pub piece_length: i64,
    pub pieces: Vec<[u8; 20]>
}

#[derive(Debug, Clone)]
pub struct ConversionError {
    pub failed_to_find: String
}

impl ConversionError {
    pub fn new(msg: &str) -> Self {
        Self { failed_to_find: msg.to_string() }
    }
}

impl Error for ConversionError {}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Failed to find [{}] field", self.failed_to_find)
    }
}

pub fn bentree_to_torrent_file<'a>(ast: &'a BP::AST<'a>) -> Result<TorrentFile<'a>, ConversionError> {
    let announce = ast.get_str(b"announce").ok_or(ConversionError::new("announce"))?;

    let Some(BP::AST::List(announce_list_raw)) = ast.get_from_dict(b"announce-list") else {
        return Err(ConversionError::new("announce_list"));
    };

    let opt_announce_list: Option<Vec<Vec<&str>>> = announce_list_raw
                                                    .into_iter()
                                                    .map(| node | node.get_list_of_str())
                                                    .collect();

    let announce_list = opt_announce_list.ok_or(ConversionError::new("announce_list"))?;

    let comment = ast.get_str(b"comment").ok_or(ConversionError::new("comment"))?;
    let created_by = ast.get_str(b"created by").ok_or(ConversionError::new("created by"))?;
    let creation_date = ast.get_int(b"creation date").ok_or(ConversionError::new("creation date"))?;
    let encoding = ast.get_str(b"encoding").ok_or(ConversionError::new("encoding"))?;
    let publisher = ast.get_str(b"publisher").ok_or(ConversionError::new("publisher"))?;
    let publisher_url = ast.get_str(b"publisher-url").ok_or(ConversionError::new("publisher-url"))?;

    let info_dict = ast.get_from_dict(b"info").ok_or(ConversionError::new("info"))?;
    let name = info_dict.get_str(b"name").ok_or(ConversionError::new(""))?;
    let piece_length = info_dict.get_int(b"piece length").ok_or(ConversionError::new("piece length"))?;

    let Some(BP::AST::ByteString(pieces_raw)) = info_dict.get_from_dict(b"pieces") else {
        return Err(ConversionError::new("pieces"));
    };

    let pieces: Vec<[u8; 20]> = pieces_raw
        .chunks_exact(20)
        .map(|c| c.try_into().unwrap())
        .collect();

    let info = TorrentInfo {
        name,
        piece_length,
        pieces
    };

    Ok(TorrentFile {
        announce,
        announce_list,
        comment,
        created_by,
        creation_date,
        encoding,
        info,
        publisher,
        publisher_url,
    })
}