use rs_torrent_client::bencoding_parser as BP;

#[test]
fn test_parsing_identity_fishing_torrent() {
    let original_bytes = include_bytes!("../samples/Wet_Fly_Fishing.torrent");
    let bytes: &mut &[u8] = &mut &original_bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");
    let constructed_bencode = ast.serialize();

    assert_eq!(original_bytes.to_vec(), constructed_bencode);
}


#[test]
fn test_parsing_identity_neuro_announce() {
    let original_bytes = include_bytes!("../samples/announce-neuro-science.bencode");
    let bytes: &mut &[u8] = &mut &original_bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");
    let constructed_bencode = ast.serialize();

    assert_eq!(original_bytes.to_vec(), constructed_bencode);
}
