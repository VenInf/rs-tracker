use torrent_reader::bencoding_parser as BP;
use torrent_reader::torrent_file as TF;

#[test]
fn test_debian_parsing() {
    let bytes = include_bytes!("../samples/debian.torrent");
    let bytes: &mut &[u8] = &mut &bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");

    let torrent = TF::bentree_to_torrent_file(&ast).expect("Conversion to TorrentFile failed");

    assert_eq!(
        torrent.announce_list,
        None
    );
    assert_eq!(
        torrent.info.name,
        "debian-13.4.0-amd64-netinst.iso"
    );
    assert_eq!(torrent.info.piece_length, 262144);
    assert_eq!(torrent.info.pieces.len(), 3016);

    assert_eq!(torrent.announce, Some("http://bttracker.debian.org:6969/announce"));
    assert_eq!(
        torrent.comment,
        Some("Debian CD from cdimage.debian.org")
    );
    assert_eq!(torrent.created_by, Some("mktorrent 1.1"));
    assert_eq!(torrent.creation_date, Some(1773496473));
    assert_eq!(torrent.encoding, None);

    let TF::FileData::Single {length} = torrent.info.file_data else { panic!("Expected a single file torrent")};
    let length_by_pieces: i64 = torrent.info.piece_length * torrent.info.pieces.len() as i64;
    assert!(length <= length_by_pieces);
    assert!(length_by_pieces - torrent.info.piece_length <= length);   }

#[test]
fn test_wet_fly_fishing_parsing() {
    let bytes = include_bytes!("../samples/Wet_Fly_Fishing.torrent");
    let bytes: &mut &[u8] = &mut &bytes[..];

    let ast = BP::parse_bencode(bytes).expect("Bencode decoding failed");

    let torrent = TF::bentree_to_torrent_file(&ast).expect("Conversion to TorrentFile failed");

    assert_eq!(
        torrent.announce_list,
        Some(vec!["http://bt3.t-ru.org/ann", "http://retracker.local/announce"])
    );
    assert_eq!(
        torrent.info.name,
        "Wet_Fly_Fishing_on_Rivers_Oliver_Edwards_[torrents.ru]"
    );
    assert_eq!(torrent.info.piece_length, 4194304);
    assert_eq!(torrent.info.pieces.len(), 880);

    assert_eq!(torrent.announce, Some("http://bt3.t-ru.org/ann"));
    assert_eq!(
        torrent.comment,
        Some("https://rutracker.org/forum/viewtopic.php?t=1786135")
    );
    assert_eq!(torrent.created_by, Some("uTorrent/1820"));
    assert_eq!(torrent.creation_date, Some(1240585607));
    assert_eq!(torrent.encoding, Some("UTF-8"));

    let TF::FileData::Multi { files } = torrent.info.file_data else { panic!("Expected a multi file torrent")};
    let length: i64 = files.iter().map(|f| f.length).sum();
    let length_by_pieces: i64 = torrent.info.piece_length * torrent.info.pieces.len() as i64;
    assert!(length <= length_by_pieces);
    assert!(length_by_pieces - torrent.info.piece_length <= length);   
}

