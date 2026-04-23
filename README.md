### What is this?

A small rust torrent client.

### What is implemented?

- [x] Can parse bittorrent files (single and multi)
- [x] Can construct bencoded messages and hash them
- [x] Can download a bittorrent file
- [x] Can respond to block requests
- [ ] Can seed the torrent file


Current TODO:
- [ ] Send the Cancel requests
- [ ] Serve for external peers
- [ ] add multifile torrent support
- [ ] add url_list support (BEP 19)
- [ ] make a proper peer id generator
- [ ] rewrite bencode decoder helpers