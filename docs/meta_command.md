# UMCTL Command
## UMCTL SETCLUSTER
UMCTL SETCLUSTER
- epoch
- flags
- [dbname1 ip:port slot_range]
- [other_dbname ip:port slot_range...]
- [PEER [dbname1 ip:port slot_range...]]
- [CONFIG [dbname1 field value...]]

Sets the mapping relationship between the server-side proxy and its corresponding redis instances behind it.

- `epoch` is the logical time of the configuration this command is sending used to decide which configuration is more up-to-date.
Every running server-side proxy will store its epoch and will reject all the `UMCTL [SETCLUSTER|SETREPL]` requests which don't have higher epoch.
- `flags`: Currently it may be NOFLAG or FORCE. When it's `FORCE`, the server-side proxy will ignore the epoch rule above and will always accept the configuration
- `slot_range` can be like
    - 1 0-1000
    - 2 0-1000 2000-3000
    - migrating 1 0-1000 epoch src_proxy_address src_node_address dst_proxy_address dst_node_address
    - importing 1 0-1000 epoch src_proxy_address src_node_address dst_proxy_address dst_node_address
- `ip:port` should be the addresses of redis instances or other proxies for `PEER` part.

Note that both these two commands set all the `local` or `peer` meta data of the proxy.
For example, you can't add multiple backend redis instances one by one by sending multiple `UMCTL SETCLUSTER`.
You should batch them in just one `UMCTL SETCLUSTER`.

## UMCTL SETREPL
UMCTL SETREPL
- epoch
- flags
- [[master|replica] dbname1 node_ip:node_port peer_num [peer_node_ip:peer_node_port peer_proxy_ip:peer_proxy_port]...] ...

Sets the replication metadata to server-side proxies. This API supports multiple replicas for a master and also multiple masters for a replica.

- For master `node_ip:node_port` is the master node. For replica it's replica node.
- `peer_node_ip:peer_node_port` is the node port of the corresponding master if we're sending this to a replica, and vice versa.
- `peer_proxy_ip:peer_proxy_port` is similar.