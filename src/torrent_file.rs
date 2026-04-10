use crate::bencoding_parser as BP;
use std::{error::Error, fmt};

// TODO: add support for the `url_list` (BEP 19)
// TODO: add getter for announces(?)

#[derive(Debug)]
pub struct TorrentFile<'a> {
    pub info: TorrentInfo<'a>,
    pub info_hash: [u8; 20],
    pub left_initial: i64,
    pub announce: Option<&'a str>,
    pub announce_list: Option<Vec<&'a str>>,
    pub url_list: Option<Vec<&'a str>>,
    pub comment: Option<&'a str>,
    pub created_by: Option<&'a str>,
    pub creation_date: Option<i64>,
    pub encoding: Option<&'a str>,
    pub publisher: Option<&'a str>,
    pub publisher_url: Option<&'a str>,
}

#[derive(Debug)]
pub struct TorrentInfo<'a> {
    pub file_data: FileData<'a>,
    pub name: &'a str,
    pub piece_length: i64,
    pub pieces: Vec<[u8; 20]>,
}

#[derive(Debug)]
pub enum FileData<'a> {
    Single { length: i64 },
    Multi { files: Vec<File<'a>> },
}

#[derive(Debug)]
pub struct File<'a> {
    pub length: i64,
    pub path: Vec<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ConversionError {
    pub failed_to_find: String,
}

impl ConversionError {
    pub fn new(msg: &str) -> Self {
        Self {
            failed_to_find: msg.to_string(),
        }
    }
}

impl Error for ConversionError {}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Failed to find [{}] field(s)", self.failed_to_find)
    }
}

pub fn bentree_to_torrent_file<'a>(
    ast: &'a BP::AST<'a>,
) -> Result<TorrentFile<'a>, ConversionError> {
    let announce = ast.get_str(b"announce");

    let info_dict = ast
        .get_from_dict(b"info")
        .ok_or(ConversionError::new("info"))?;
    let name = info_dict
        .get_str(b"name")
        .ok_or(ConversionError::new("name"))?;
    let piece_length = info_dict
        .get_int(b"piece length")
        .ok_or(ConversionError::new("piece length"))?;
    let Some(BP::AST::ByteString(pieces_raw)) = info_dict.get_from_dict(b"pieces") else {
        return Err(ConversionError::new("pieces"));
    };

    let pieces: Vec<[u8; 20]> = pieces_raw
        .chunks_exact(20)
        .map(|c| c.try_into().unwrap())
        .collect();

    let file_data = if let Some(length) = info_dict.get_int(b"length") {
        FileData::Single { length }
    } else {
        if let Some(BP::AST::List(files_list)) = info_dict.get_from_dict(b"files") {
            let opt_files: Option<Vec<File>> = files_list
                .into_iter()
                .map(|node| {
                    Some(File {
                        length: node.get_int(b"length")?,
                        path: node.get_from_dict(b"path")?.get_list_of_str()?,
                    })
                })
                .collect();

            let files = opt_files.ok_or(ConversionError::new("files"))?;
            FileData::Multi { files }
        } else {
            return Err(ConversionError::new("length | files"));
        }
    };

    let left_initial: i64 = match file_data {
        FileData::Single { length } => length,
        FileData::Multi { ref files } => files.iter().map(|file| file.length).sum(),
    };

    let info = TorrentInfo {
        name,
        piece_length,
        pieces,
        file_data,
    };

    let announce_list: Option<Vec<&str>> = ast
        .get_from_dict(b"announce-list")
        .and_then(|node| node.get_list_of_list_of_str());
    let url_list: Option<Vec<&str>> = ast
        .get_from_dict(b"url-list")
        .and_then(|node| node.get_list_of_str());

    let comment = ast.get_str(b"comment");
    let created_by = ast.get_str(b"created by");
    let creation_date = ast.get_int(b"creation date");
    let encoding = ast.get_str(b"encoding");
    let publisher = ast.get_str(b"publisher");
    let publisher_url = ast.get_str(b"publisher-url");

    Ok(TorrentFile {
        info,
        info_hash: info_dict.hash(),
        left_initial,
        announce,
        announce_list,
        url_list,
        comment,
        created_by,
        creation_date,
        encoding,
        publisher,
        publisher_url,
    })
}
