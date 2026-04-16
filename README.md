### What is this?

A small rust torrent tracker, work in progress.

### What is implemented?

- [x] Can parse bittorrent files (single and multi)
- [x] Can construct bencoded messages and hash them
- [ ] Can download a bittorrent file
    - [x] Can send and receive the announcment
    - [x] Can do a handshake
    - [ ] Can do the bitfields
    - [x] Can do the piece request
    - [ ] Can send a Have message to all other peers
- [ ] Can seed the torrent file

Jobs queue:
    Arc<Mutex<T>>
    parts_to_download
    parts_downloaded

Peers:
    one tokio task per peer

Peer:
    chocke-unchoke management in the download functions
    full lifecycle run_peer
    local queue of parts to download, send one part requests in one batch  
    handle all possible error gracefully, return failed pulls back to the main queue


Current TODO:
- [ ] add batching
- [ ] make a job queue
- [ ] make support for multiple peers
- [ ] add multifile torrent support
- [ ] refactor str and String usages
- [ ] make a proper peer id generator
- [ ] rewrite bencode decoder helpers
- [ ] add url_list support