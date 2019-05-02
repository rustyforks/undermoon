use common::cluster::ReplPeer;
use common::db::DBMapFlags;
use common::utils::{CmdParseError, ThreadSafe};
use futures::Future;
use protocol::RedisClientError;
use protocol::{Array, BulkStr, Resp};
use std::error::Error;
use std::fmt;
use std::io;
use std::str;

// MasterReplicator and ReplicaReplicator work together remotely to manage the replication.

pub trait MasterReplicator: ThreadSafe {
    fn start(&self) -> Box<dyn Future<Item = (), Error = ReplicatorError> + Send>;
    fn stop(&self) -> Box<dyn Future<Item = (), Error = ReplicatorError> + Send>;
    fn get_meta(&self) -> &MasterMeta;
}

pub trait ReplicaReplicator: ThreadSafe {
    fn start(&self) -> Box<dyn Future<Item = (), Error = ReplicatorError> + Send>;
    fn stop(&self) -> Box<dyn Future<Item = (), Error = ReplicatorError> + Send>;
    fn get_meta(&self) -> &ReplicaMeta;
}

#[derive(Debug, Clone)]
pub struct ReplicatorMeta {
    pub epoch: u64,
    pub flags: DBMapFlags,
    pub masters: Vec<MasterMeta>,
    pub replicas: Vec<ReplicaMeta>,
}

impl ReplicatorMeta {
    pub fn from_resp(resp: &Resp) -> Result<Self, CmdParseError> {
        parse_repl_meta(resp)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct MasterMeta {
    pub db_name: String,
    pub master_node_address: String,
    pub replicas: Vec<ReplPeer>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct ReplicaMeta {
    pub db_name: String,
    pub replica_node_address: String,
    pub masters: Vec<ReplPeer>,
}

fn parse_repl_meta(resp: &Resp) -> Result<ReplicatorMeta, CmdParseError> {
    let arr = match resp {
        Resp::Arr(Array::Arr(ref arr)) => arr,
        _ => return Err(CmdParseError {}),
    };

    // Skip the "UMCTL SETREPL"
    let it = arr.iter().skip(2).flat_map(|resp| match resp {
        Resp::Bulk(BulkStr::Str(safe_str)) => match str::from_utf8(safe_str) {
            Ok(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    });
    let mut it = it.peekable();

    let epoch_str = it.next().ok_or(CmdParseError {})?;
    let epoch = epoch_str.parse::<u64>().map_err(|_e| CmdParseError {})?;

    let flags = DBMapFlags::from_arg(&it.next().ok_or(CmdParseError {})?);

    let mut master_meta_array = Vec::new();
    let mut replica_meta_array = Vec::new();

    while it.peek().is_some() {
        let mut peers = Vec::new();

        let role = it.next().ok_or(CmdParseError {})?;
        let db_name = it.next().ok_or(CmdParseError {})?;
        let node_address = it.next().ok_or(CmdParseError {})?;
        let peer_num = it
            .next()
            .ok_or(CmdParseError {})?
            .parse::<usize>()
            .map_err(|_| CmdParseError {})?;
        for _ in 0..peer_num {
            let node_address = it.next().ok_or(CmdParseError {})?;
            let proxy_address = it.next().ok_or(CmdParseError {})?;
            peers.push(ReplPeer {
                node_address,
                proxy_address,
            })
        }

        if role.to_uppercase() == "MASTER" {
            master_meta_array.push(MasterMeta {
                db_name,
                master_node_address: node_address,
                replicas: peers,
            })
        } else if role.to_uppercase() == "REPLICA" {
            replica_meta_array.push(ReplicaMeta {
                db_name,
                replica_node_address: node_address,
                masters: peers,
            })
        } else {
            error!("invalid role {}", role);
            return Err(CmdParseError {});
        }
    }

    Ok(ReplicatorMeta {
        epoch,
        flags,
        masters: master_meta_array,
        replicas: replica_meta_array,
    })
}

pub fn encode_repl_meta(meta: ReplicatorMeta) -> Vec<String> {
    let ReplicatorMeta {
        epoch,
        flags,
        masters,
        replicas,
    } = meta;

    let mut args = Vec::new();
    args.push(epoch.to_string());
    args.push(flags.to_arg());

    for master in masters.iter() {
        args.push("master".to_string());
        args.push(master.db_name.clone());
        args.push(master.master_node_address.clone());
        args.push(master.replicas.len().to_string());
        for replica in master.replicas.iter() {
            args.push(replica.node_address.clone());
            args.push(replica.proxy_address.clone());
        }
    }
    for replica in replicas.iter() {
        args.push("replica".to_string());
        args.push(replica.db_name.clone());
        args.push(replica.replica_node_address.clone());
        args.push(replica.masters.len().to_string());
        for master in replica.masters.iter() {
            args.push(master.node_address.clone());
            args.push(master.proxy_address.clone());
        }
    }

    args
}

#[derive(Debug)]
pub enum ReplicatorError {
    IncompatibleVersion,
    InvalidAddress,
    AlreadyStarted,
    AlreadyEnded,
    Canceled,
    RedisError(RedisClientError),
    Io(io::Error),
}

impl fmt::Display for ReplicatorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for ReplicatorError {
    fn description(&self) -> &str {
        "replicator error"
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            ReplicatorError::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_encode_single_replicator() {
        let arguments =
            "UMCTL SETREPL 233 force master testdb localhost:6000 1 localhost:6001 localhost:5299"
                .split(' ')
                .map(|s| Resp::Bulk(BulkStr::Str(s.to_string().into_bytes())))
                .collect();
        let resp = Resp::Arr(Array::Arr(arguments));
        let r = parse_repl_meta(&resp);
        assert!(r.is_ok());
        let meta = r.expect("not success");
        assert_eq!(meta.epoch, 233);
        assert_eq!(meta.flags, DBMapFlags { force: true });
        assert_eq!(meta.masters.len(), 1);
        assert_eq!(meta.replicas.len(), 0);

        let args = encode_repl_meta(meta.clone()).join(" ");
        assert_eq!(
            args,
            "233 FORCE master testdb localhost:6000 1 localhost:6001 localhost:5299"
        );
    }

    #[test]
    fn test_parse_and_encode_multi_replicators() {
        let arguments = "UMCTL SETREPL 233 noflag master testdb localhost:6000 1 localhost:6001 localhost:5299 replica testdb localhost:6001 1 localhost:6000 localhost:5299"
            .split(' ')
            .map(|s| Resp::Bulk(BulkStr::Str(s.to_string().into_bytes())))
            .collect();
        let resp = Resp::Arr(Array::Arr(arguments));
        let r = parse_repl_meta(&resp);
        assert!(r.is_ok());
        let meta = r.expect("not success");
        assert_eq!(meta.epoch, 233);
        assert_eq!(meta.flags, DBMapFlags { force: false });
        assert_eq!(meta.masters.len(), 1);
        assert_eq!(meta.replicas.len(), 1);

        let master = &meta.masters[0];
        assert_eq!(master.db_name, "testdb");
        assert_eq!(master.master_node_address, "localhost:6000");
        assert_eq!(master.replicas.len(), 1);
        assert_eq!(master.replicas[0].node_address, "localhost:6001");
        assert_eq!(master.replicas[0].proxy_address, "localhost:5299");

        let replica = &meta.replicas[0];
        assert_eq!(replica.db_name, "testdb");
        assert_eq!(replica.replica_node_address, "localhost:6001");
        assert_eq!(replica.masters.len(), 1);
        assert_eq!(replica.masters[0].node_address, "localhost:6000");
        assert_eq!(replica.masters[0].proxy_address, "localhost:5299");

        let args = encode_repl_meta(meta.clone()).join(" ");
        assert_eq!(args, "233 NOFLAG master testdb localhost:6000 1 localhost:6001 localhost:5299 replica testdb localhost:6001 1 localhost:6000 localhost:5299")
    }

}
