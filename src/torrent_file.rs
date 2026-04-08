use crate::bencoding_parser as BP;
use std::{error::Error, fmt};

#[derive(Debug)]
pub struct TorrentTorrentFile<'a> {
    pub announce: &'a str,
    pub info: TorrentInfo<'a>,
    pub announce_list: Option<Vec<Vec<&'a str>>>,
    pub comment:  Option<&'a str>,
    pub created_by:  Option<&'a str>,
    pub creation_date:  Option<i64>,
    pub encoding:  Option<&'a str>,
    pub publisher:  Option<&'a str>,
    pub publisher_url:  Option<&'a str>,
}


// TODO: Add single file mode
#[derive(Debug)]
pub struct TorrentInfo<'a> {
    pub files: Vec<TorrentFile<'a>>,
    pub piece_length: i64,
    pub pieces: Vec<[u8; 20]>,
    pub name: &'a str,
}

#[derive(Debug)]
pub struct TorrentFile<'a> {
    pub length: i64,
    pub path: Vec<&'a str>,
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

pub fn bentree_to_torrent_file<'a>(ast: &'a BP::AST<'a>) -> Result<TorrentTorrentFile<'a>, ConversionError> {
    let announce = ast.get_str(b"announce").ok_or(ConversionError::new("announce"))?;


    let info_dict = ast.get_from_dict(b"info").ok_or(ConversionError::new("info"))?;
    let piece_length = info_dict.get_int(b"piece length").ok_or(ConversionError::new("piece length"))?;
    let name = info_dict.get_str(b"name").ok_or(ConversionError::new("name"))?;

    let Some(BP::AST::List(files_list)) = info_dict.get_from_dict(b"files") else {
        return Err(ConversionError::new("files"));
    };
    
    let opt_files: Option<Vec<TorrentFile>> = files_list
                                             .into_iter()
                                             .map(| node | Some(TorrentFile {
                                                           length: node.get_int(b"length")?,
                                                           path: node.get_from_dict(b"path")?.get_list_of_str()?
                                             }))
                                             .collect();

    let files = opt_files.ok_or(ConversionError::new("files"))?;

    let Some(BP::AST::ByteString(pieces_raw)) = info_dict.get_from_dict(b"pieces") else {
        return Err(ConversionError::new("pieces"));
    };

    let pieces: Vec<[u8; 20]> = pieces_raw
        .chunks_exact(20)
        .map(|c| c.try_into().unwrap())
        .collect();

    let info = TorrentInfo {
        files,
        name,
        piece_length,
        pieces,
    };

    let Some(BP::AST::List(announce_list_raw)) = ast.get_from_dict(b"announce-list") else {
        return Err(ConversionError::new("announce_list"));
    };
    let announce_list: Option<Vec<Vec<&str>>> = announce_list_raw
                                                .into_iter()
                                                .map(| node | node.get_list_of_str())
                                                .collect();
    
    let comment = ast.get_str(b"comment");
    let created_by = ast.get_str(b"created by");
    let creation_date = ast.get_int(b"creation date");
    let encoding = ast.get_str(b"encoding");
    let publisher = ast.get_str(b"publisher");
    let publisher_url = ast.get_str(b"publisher-url");

    Ok(TorrentTorrentFile {
        announce,
        info,
        announce_list,
        comment,
        created_by,
        creation_date,
        encoding,
        publisher,
        publisher_url,
    })
}