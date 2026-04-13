

#[derive(Debug, Clone)]
pub struct Pieces;


#[derive(Debug)]
pub struct Piece<'a> {
    pub piece_hash: [u8; 20],
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

