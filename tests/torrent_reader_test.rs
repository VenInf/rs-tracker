use torrent_reader::bencoding_parser as BP;

#[test]
fn test_parsing_identity() {
    let original_bytes = include_bytes!("../samples/Wet_Fly_Fishing.torrent");
    let bytes: &mut &[u8] = &mut &original_bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");
    let constructed_bencode = ast.construct_bencode();

    assert_eq!(original_bytes.to_vec(), constructed_bencode);
}
