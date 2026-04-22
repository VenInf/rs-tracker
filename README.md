### What is this?

A small rust torrent tracker, work in progress.

### What is implemented?

- [x] Can parse bittorrent files (single and multi)
- [x] Can construct bencoded messages and hash them
- [ ] Can download a bittorrent file
    - [x] Can send and receive the announcment
    - [x] Can do a handshake
    - [x] Can do the bitfields
    - [x] Can do the piece request
    - [x] Can send a Have message to all other peers
- [ ] Can seed the torrent file

Current TODO:
- [x] add batching
- [x] make a job queue
- [x] make support for multiple peers
- [ ] Send the Cancel requests
- [ ] add multifile torrent support
- [x] refactor str and String usages
- [ ] make a proper peer id generator
- [ ] rewrite bencode decoder helpers
- [ ] add url_list support