#[cfg(test)]
mod tests {
    use torrent_reader::pieces::Bitfield;

    #[test]
    fn test_bitfield_basic_set_has() {
        let mut bf = Bitfield { bytes: vec![0, 0] };

        assert!(!bf.has(0));
        assert!(!bf.has(7));
        assert!(!bf.has(8));

        bf.set(0);
        assert!(bf.has(0), "Piece 0 should be set");
        assert_eq!(bf.bytes[0], 0b1000_0000, "First byte should have MSB set");

        bf.set(11);
        assert!(bf.has(11));
        assert_eq!(bf.bytes[1], 0b0001_0000);
    }

    #[test]
    fn test_bitfield_byte_boundaries() {
        let mut bf = Bitfield { bytes: vec![0] };

        bf.set(0);
        bf.set(7);

        assert!(bf.has(0));
        assert!(bf.has(7));
        assert!(!bf.has(3));

        assert_eq!(bf.bytes[0], 129);
    }

    #[test]
    fn test_set_all_and_has() {
        let mut bf = Bitfield { bytes: vec![255] };

        bf.set_all(&vec![1, 1, 1, 1, 1, 1, 1, 1]);

        assert!(bf.has(0));
        assert!(!bf.has(1));
        assert!(bf.has(2));
        assert!(!bf.has(3));
        assert!(bf.has(4));
        assert!(!bf.has(5));
        assert!(bf.has(6));
        assert!(!bf.has(7));
    }

    #[test]
    fn test_out_of_bounds_safety() {
        let mut bf = Bitfield { bytes: vec![0] };

        // Testing 'has' beyond the vector length should not panic
        assert!(!bf.has(100));

        // Testing 'set' beyond the vector length should not panic
        bf.set(100);
        assert_eq!(bf.bytes.len(), 1, "Length should not have changed");
    }
}
