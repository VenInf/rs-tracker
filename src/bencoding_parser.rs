use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::fmt;
use winnow::ascii;
use winnow::combinator as comb;
use winnow::error::{ContextError, Result, StrContext};
use winnow::token;
use winnow::{self, Parser};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AST<'a> {
    ByteString(&'a [u8]),
    Integer(i64),
    List(Vec<AST<'a>>),
    Dictionary(BTreeMap<AST<'a>, AST<'a>>),
}

impl<'a> AST<'a> {
    fn fmt_with_indent(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        let pad = "  ".repeat(indent);

        match self {
            AST::ByteString(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    write!(f, "\"{}\"", s)
                } else {
                    write!(f, "<HASHES>")
                }
            }
            AST::Integer(i) => write!(f, "{}", i),

            AST::List(items) => {
                if items.is_empty() {
                    return write!(f, "[]");
                }
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    item.fmt_with_indent(f, indent + 1)?;
                    if i < items.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, "]")
            }

            AST::Dictionary(dict) => {
                if dict.is_empty() {
                    return write!(f, "{{}}");
                }
                writeln!(f, "{{")?;
                for (i, (key, val)) in dict.iter().enumerate() {
                    write!(f, "  {}", pad)?;
                    key.fmt_with_indent(f, indent + 1)?;
                    write!(f, " => ")?;
                    val.fmt_with_indent(f, indent + 1)?;

                    if i < dict.len() - 1 {
                        writeln!(f, ",")?;
                    } else {
                        writeln!(f)?;
                    }
                }
                write!(f, "{}}}", pad)
            }
        }
    }
}

impl<'a> fmt::Display for AST<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with_indent(f, 0)
    }
}

impl<'a> AST<'a> {
    pub fn serialize(&'a self) -> Vec<u8> {
        match self {
            AST::ByteString(bs) => {
                let len_bytes: Vec<u8> = bs.len().to_string().into_bytes();
                [len_bytes, b":".to_vec(), bs.to_vec()].concat()
            }
            AST::Integer(i) => {
                let i_bytes: Vec<u8> = i.to_string().into_bytes();
                [b"i".to_vec(), i_bytes, b"e".to_vec()].concat()
            }
            AST::List(l) => {
                let list_bytes: Vec<u8> = l.iter().flat_map(|node| node.serialize()).collect();
                [b"l".to_vec(), list_bytes, b"e".to_vec()].concat()
            }
            AST::Dictionary(d) => {
                let mut dict_bytes = Vec::new();
                for (key, value) in d {
                    dict_bytes.extend(key.serialize());
                    dict_bytes.extend(value.serialize());
                }
                [b"d".to_vec(), dict_bytes, b"e".to_vec()].concat()
            }
        }
    }

    pub fn hash(&'a self) -> [u8; 20] {
        let bencode = self.serialize();
        Sha1::digest(bencode).into()
    }

    // TODO: rewrite this???
    pub fn get_from_dict(&'a self, key: &'a [u8]) -> Option<&'a AST<'a>> {
        match self {
            AST::Dictionary(map) => map.get(&AST::ByteString(key)),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        if let AST::Integer(i) = self {
            return Some(*i);
        }
        None
    }

    pub fn get_int(&self, key: &'a [u8]) -> Option<i64> {
        if let Some(AST::Integer(i)) = self.get_from_dict(key) {
            return Some(*i);
        }
        None
    }

    pub fn str(&self) -> Option<String> {
        if let AST::ByteString(bs) = self {
            return String::from_utf8(bs.to_vec()).ok();
        }
        None
    }

    pub fn get_str(&'a self, key: &'a [u8]) -> Option<String> {
        if let Some(AST::ByteString(b)) = self.get_from_dict(key) {
            return String::from_utf8(b.to_vec()).ok();
        }
        None
    }

    pub fn list(&'a self) -> Option<&'a Vec<AST<'a>>> {
        if let AST::List(l) = self {
            return Some(l);
        }
        None
    }

    pub fn get_list_of_str(&'a self) -> Option<Vec<String>> {
        self.list()
            .and_then(|l| l.iter().map(|bs| bs.str()).collect())
    }

    pub fn get_list_of_list_of_str(&'a self) -> Option<Vec<String>> {
        self.list().map(|outer| {
            outer
                .iter()
                .filter_map(|inner| inner.get_list_of_str())
                .flatten()
                .collect()
        })
    }
}

// Parsing Bencoded u8 slice to the AST
pub fn parse_bencode<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    comb::alt((
        parse_integer.context(StrContext::Label("Parsing an Integer")),
        parse_bytestring.context(StrContext::Label("Parsing a ByteString")),
        parse_list.context(StrContext::Label("Parsing a List")),
        parse_dictionary.context(StrContext::Label("Parsing a Dictionary")),
    ))
    .parse_next(input_slice)
}

// ByteStrings

// Byte strings are encoded as follows: <string length encoded in base ten ASCII>:<string data>
// Note that there is no constant beginning delimiter, and no ending delimiter.

//     Example: 4:spam represents the string "spam"
//     Example: 0: represents the empty string ""

fn parse_bytestring<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    let num: u64 = ascii::dec_uint(input_slice)?;
    token::literal(':').parse_next(input_slice)?;
    let n_tokens = token::take(num).parse_next(input_slice)?;

    Ok(AST::ByteString(n_tokens))
}

// Integers

// Integers are encoded as follows: i<integer encoded in base ten ASCII>e
// The initial i and trailing e are beginning and ending delimiters.

//     Example: i3e represents the integer "3"
//     Example: i-3e represents the integer "-3"

// i-0e is invalid. All encodings with a leading zero, such as i03e, are invalid, other than i0e, which of course corresponds to the integer "0".

//     NOTE: The maximum number of bit of this integer is unspecified, but to handle it as a signed 64bit integer is mandatory to handle "large files" aka .torrent for more that 4Gbyte.

fn parse_integer<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    token::literal('i').parse_next(input_slice)?;
    let int: i64 = ascii::dec_int(input_slice)?;
    token::literal('e').parse_next(input_slice)?;

    Ok(AST::Integer(int))
}

// Lists

// Lists are encoded as follows: l<bencoded values>e
// The initial l and trailing e are beginning and ending delimiters. Lists may contain any bencoded type, including integers, strings, dictionaries, and even lists within other lists.

//     Example: l4:spam4:eggse represents the list of two strings: [ "spam", "eggs" ]
//     Example: le represents an empty list: []

fn parse_list<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    token::literal('l').parse_next(input_slice)?;

    let res = comb::repeat_till(0.., parse_bencode, 'e').parse_next(input_slice)?;
    Ok(AST::List(res.0))
}

// Dictionaries

// Dictionaries are encoded as follows: d<bencoded string><bencoded element>e
// The initial d and trailing e are the beginning and ending delimiters. Note that the keys must be bencoded strings. The values may be any bencoded type, including integers, strings, lists, and other dictionaries. Keys must be strings and appear in sorted order (sorted as raw strings, not alphanumerics). The strings should be compared using a binary comparison, not a culture-specific "natural" comparison.

//     Example: d3:cow3:moo4:spam4:eggse represents the dictionary { "cow" => "moo", "spam" => "eggs" }
//     Example: d4:spaml1:a1:bee represents the dictionary { "spam" => [ "a", "b" ] }
//     Example: d9:publisher3:bob17:publisher-webpage15:www.example.com18:publisher.location4:homee represents { "publisher" => "bob", "publisher-webpage" => "www.example.com", "publisher.location" => "home" }
//     Example: de represents an empty dictionary {}

fn parse_dictionary<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    token::literal('d').parse_next(input_slice)?;

    let res: (BTreeMap<AST, AST>, char) =
        comb::repeat_till(0.., (parse_bytestring, parse_bencode), 'e').parse_next(input_slice)?;
    Ok(AST::Dictionary(res.0))
}
