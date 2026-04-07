use winnow::error::AddContext;
use winnow::token as token;
use winnow::ascii as ascii;
use winnow::combinator as comb;
use winnow::{self, Parser};
use winnow::error::{ContextError, Result, StrContext};
use std::collections::BTreeMap;
use std::fmt;

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

impl<'a> std::fmt::Display for AST<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_with_indent(f, 0)
    }
}

// ByteStrings

// Byte strings are encoded as follows: <string length encoded in base ten ASCII>:<string data>
// Note that there is no constant beginning delimiter, and no ending delimiter.

//     Example: 4:spam represents the string "spam"
//     Example: 0: represents the empty string ""

pub fn parse_bytestring<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
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

pub fn parse_integer<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
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

pub fn parse_list<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
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

pub fn parse_dictionary<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    token::literal('d').parse_next(input_slice)?;    

    let res: (BTreeMap<AST, AST>, char) = comb::repeat_till(0.., (parse_bytestring, parse_bencode), 'e').parse_next(input_slice)?;
    Ok(AST::Dictionary(res.0))
}

pub fn parse_bencode<'a>(input_slice: &mut &'a [u8]) -> Result<AST<'a>, ContextError> {
    comb::alt((
        parse_integer.context(StrContext::Label("Parsing an Integer")),
        parse_bytestring.context(StrContext::Label("Parsing a ByteString")),
        parse_list.context(StrContext::Label("Parsing a List")),
        parse_dictionary.context(StrContext::Label("Parsing a Dictionary")),
    )).parse_next(input_slice)
}
