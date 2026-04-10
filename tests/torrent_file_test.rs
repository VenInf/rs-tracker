use torrent_reader::torrent_file as TF;
use torrent_reader::bencoding_parser as BP;

#[test]
fn test_wet_fly_fishing_parsing() {
    let bytes = include_bytes!("../samples/Wet_Fly_Fishing.torrent");
    let bytes: &mut &[u8] = &mut &bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");

    let torrent = TF::bentree_to_torrent_file(&ast).expect("Conversion to TorrentFile failed");
    
    assert_eq!(torrent.announce_list,  vec!["http://bt3.t-ru.org/ann", "http://retracker.local/announce"]);
    assert_eq!(torrent.info.name, "Wet_Fly_Fishing_on_Rivers_Oliver_Edwards_[torrents.ru]");
    assert_eq!(torrent.info.piece_length, 4194304);
    assert_eq!(torrent.info.pieces.len(), 880);    
    
    assert_eq!(torrent.announce, Some("http://bt3.t-ru.org/ann"));
    assert_eq!(torrent.comment, Some("https://rutracker.org/forum/viewtopic.php?t=1786135"));
    assert_eq!(torrent.created_by, Some("uTorrent/1820"));
    assert_eq!(torrent.creation_date, Some(1240585607));
    assert_eq!(torrent.encoding, Some("UTF-8"));
}
    