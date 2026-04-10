### What is this?

A small rust torrent tracker, work in progress.

### What is implemented?

- [x] Can parse bittorrent files (single and multi)
- [x] Can construct bencoded messages and hash them
- [ ] Can download a bittorrent file
    - [ ] Can send and receive the announcment
    - [ ] Can do a handshake
    - [ ] Can do the bitfields
    - [ ] Can do the piece request
    - [ ] Can send a Have message to all other peers
- [ ] Can seed the torrent file
