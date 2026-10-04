//! # StateFS Lock-Free Mutation Stream Adapter (`bbqueue`)
//!
//! Provides a lock-free Single-Producer Single-Consumer (SPSC) continuous
//! BipBuffer for high-frequency write-ahead logging (WAL), state event streaming,
//! and background replication.
//!
//! ## Architectural Role
//!
//! In high-frequency simulations, physics loops, or gaming server ticks (e.g. GoldSrc),
//! state modifications cannot block on mutexes, I/O, or dynamic heap allocations.
//!
//! This adapter uses BipBuffers via [`bbqueue`] to grant contiguous write memory
//! directly to the producer. The producer writes framed binary events with zero allocations,
//! while a consumer on a separate background thread drains and persists or replicates
//! mutations.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use bbqueue::nicknames::Barbacoa;
use bbqueue::prod_cons::framed::{FramedConsumer, FramedProducer};
use bbqueue::traits::coordination::ReadGrantError;
use bbqueue::traits::storage::Inline;
use core::fmt;
use statefs_core::{MemStore, Path, Store, Value};

/// Default WAL buffer capacity: 64 KiB.
pub const DEFAULT_WAL_BUFFER_SIZE: usize = 65536;

/// Error variants encountered during WAL production or consumption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalError {
    /// The ringbuffer is full; write grant cannot be satisfied.
    BufferFull,
    /// Corrupted or invalid binary frame header/payload.
    MalformedFrame,
    /// Invalid UTF-8 path string in frame.
    InvalidPathEncoding,
    /// Failed to apply mutation to target store.
    StoreFailure,
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferFull => write!(f, "WAL ringbuffer is full"),
            Self::MalformedFrame => write!(f, "Malformed or truncated binary frame"),
            Self::InvalidPathEncoding => write!(f, "Invalid UTF-8 in WAL path"),
            Self::StoreFailure => write!(f, "Failed to apply WAL mutation to store"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for WalError {}

/// An operation recorded in the write-ahead log.
#[derive(Debug, Clone, PartialEq)]
pub enum WalOp {
    /// Node inserted or updated.
    Insert { path: String, value: Value },
    /// Node removed.
    Delete { path: String },
}

/// A structured mutation event decoded from the WAL stream.
#[derive(Debug, Clone, PartialEq)]
pub struct WalEvent {
    /// Monotonically increasing state revision.
    pub revision: u64,
    /// The state operation.
    pub op: WalOp,
}

const OP_INSERT: u8 = 1;
const OP_DELETE: u8 = 2;

const TAG_NULL: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_BYTES: u8 = 5;
const TAG_ARRAY: u8 = 6;
const TAG_MAP: u8 = 7;

fn calculate_value_size(val: &Value) -> usize {
    1 + match val {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Int(_) | Value::Float(_) => 8,
        Value::String(s) => 4 + s.len(),
        Value::Bytes(b) => 4 + b.len(),
        Value::Array(items) => 4 + items.iter().map(calculate_value_size).sum::<usize>(),
        Value::Map(entries) => {
            4 + entries
                .iter()
                .map(|(k, v)| 2 + k.len() + calculate_value_size(v))
                .sum::<usize>()
        }
    }
}

fn calculate_frame_size(path_bytes: usize, value: Option<&Value>) -> usize {
    // [op: 1] + [rev: 8] + [path_len: 2] + [path]
    let base = 1 + 8 + 2 + path_bytes;
    if let Some(val) = value {
        base + calculate_value_size(val)
    } else {
        base
    }
}

fn encode_value_into(val: &Value, buf: &mut [u8], offset: &mut usize) {
    match val {
        Value::Null => {
            buf[*offset] = TAG_NULL;
            *offset += 1;
        }
        Value::Bool(b) => {
            buf[*offset] = TAG_BOOL;
            buf[*offset + 1] = if *b { 1 } else { 0 };
            *offset += 2;
        }
        Value::Int(i) => {
            buf[*offset] = TAG_INT;
            *offset += 1;
            buf[*offset..*offset + 8].copy_from_slice(&i.to_le_bytes());
            *offset += 8;
        }
        Value::Float(f) => {
            buf[*offset] = TAG_FLOAT;
            *offset += 1;
            buf[*offset..*offset + 8].copy_from_slice(&f.to_le_bytes());
            *offset += 8;
        }
        Value::String(s) => {
            buf[*offset] = TAG_STRING;
            *offset += 1;
            let s_bytes = s.as_bytes();
            let s_len = s_bytes.len() as u32;
            buf[*offset..*offset + 4].copy_from_slice(&s_len.to_le_bytes());
            *offset += 4;
            buf[*offset..*offset + s_bytes.len()].copy_from_slice(s_bytes);
            *offset += s_bytes.len();
        }
        Value::Bytes(b) => {
            buf[*offset] = TAG_BYTES;
            *offset += 1;
            let b_len = b.len() as u32;
            buf[*offset..*offset + 4].copy_from_slice(&b_len.to_le_bytes());
            *offset += 4;
            buf[*offset..*offset + b.len()].copy_from_slice(b);
            *offset += b.len();
        }
        Value::Array(items) => {
            buf[*offset] = TAG_ARRAY;
            *offset += 1;
            let count = items.len() as u32;
            buf[*offset..*offset + 4].copy_from_slice(&count.to_le_bytes());
            *offset += 4;
            for item in items {
                encode_value_into(item, buf, offset);
            }
        }
        Value::Map(entries) => {
            buf[*offset] = TAG_MAP;
            *offset += 1;
            let count = entries.len() as u32;
            buf[*offset..*offset + 4].copy_from_slice(&count.to_le_bytes());
            *offset += 4;
            for (k, v) in entries {
                let k_bytes = k.as_bytes();
                let k_len = k_bytes.len() as u16;
                buf[*offset..*offset + 2].copy_from_slice(&k_len.to_le_bytes());
                *offset += 2;
                buf[*offset..*offset + k_bytes.len()].copy_from_slice(k_bytes);
                *offset += k_bytes.len();
                encode_value_into(v, buf, offset);
            }
        }
    }
}

fn decode_value_from(buf: &[u8], offset: &mut usize) -> Result<Value, WalError> {
    if buf.len() < *offset + 1 {
        return Err(WalError::MalformedFrame);
    }
    let tag = buf[*offset];
    *offset += 1;

    match tag {
        TAG_NULL => Ok(Value::Null),
        TAG_BOOL => {
            if buf.len() < *offset + 1 {
                return Err(WalError::MalformedFrame);
            }
            let b = buf[*offset] != 0;
            *offset += 1;
            Ok(Value::Bool(b))
        }
        TAG_INT => {
            if buf.len() < *offset + 8 {
                return Err(WalError::MalformedFrame);
            }
            let i = i64::from_le_bytes(
                buf[*offset..*offset + 8]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            );
            *offset += 8;
            Ok(Value::Int(i))
        }
        TAG_FLOAT => {
            if buf.len() < *offset + 8 {
                return Err(WalError::MalformedFrame);
            }
            let f = f64::from_le_bytes(
                buf[*offset..*offset + 8]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            );
            *offset += 8;
            Ok(Value::Float(f))
        }
        TAG_STRING => {
            if buf.len() < *offset + 4 {
                return Err(WalError::MalformedFrame);
            }
            let s_len = u32::from_le_bytes(
                buf[*offset..*offset + 4]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            ) as usize;
            *offset += 4;
            if buf.len() < *offset + s_len {
                return Err(WalError::MalformedFrame);
            }
            let s = core::str::from_utf8(&buf[*offset..*offset + s_len])
                .map_err(|_| WalError::InvalidPathEncoding)?
                .to_string();
            *offset += s_len;
            Ok(Value::String(s))
        }
        TAG_BYTES => {
            if buf.len() < *offset + 4 {
                return Err(WalError::MalformedFrame);
            }
            let b_len = u32::from_le_bytes(
                buf[*offset..*offset + 4]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            ) as usize;
            *offset += 4;
            if buf.len() < *offset + b_len {
                return Err(WalError::MalformedFrame);
            }
            let b = buf[*offset..*offset + b_len].to_vec();
            *offset += b_len;
            Ok(Value::Bytes(b))
        }
        TAG_ARRAY => {
            if buf.len() < *offset + 4 {
                return Err(WalError::MalformedFrame);
            }
            let count = u32::from_le_bytes(
                buf[*offset..*offset + 4]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            ) as usize;
            *offset += 4;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(decode_value_from(buf, offset)?);
            }
            Ok(Value::Array(items))
        }
        TAG_MAP => {
            if buf.len() < *offset + 4 {
                return Err(WalError::MalformedFrame);
            }
            let count = u32::from_le_bytes(
                buf[*offset..*offset + 4]
                    .try_into()
                    .map_err(|_| WalError::MalformedFrame)?,
            ) as usize;
            *offset += 4;
            let mut map = BTreeMap::new();
            for _ in 0..count {
                if buf.len() < *offset + 2 {
                    return Err(WalError::MalformedFrame);
                }
                let k_len = u16::from_le_bytes(
                    buf[*offset..*offset + 2]
                        .try_into()
                        .map_err(|_| WalError::MalformedFrame)?,
                ) as usize;
                *offset += 2;
                if buf.len() < *offset + k_len {
                    return Err(WalError::MalformedFrame);
                }
                let k = core::str::from_utf8(&buf[*offset..*offset + k_len])
                    .map_err(|_| WalError::InvalidPathEncoding)?
                    .to_string();
                *offset += k_len;
                let val = decode_value_from(buf, offset)?;
                map.insert(k, val);
            }
            Ok(Value::Map(map))
        }
        _ => Err(WalError::MalformedFrame),
    }
}

/// SPSC Producer half for publishing mutations to the lock-free WAL ringbuffer.
pub struct WalProducer<const CAP: usize = DEFAULT_WAL_BUFFER_SIZE> {
    producer: FramedProducer<
        alloc::sync::Arc<
            bbqueue::BBQueue<
                Inline<CAP>,
                bbqueue::traits::coordination::cas::AtomicCoord,
                bbqueue::traits::notifier::polling::Polling,
            >,
        >,
        u16,
    >,
}

/// SPSC Consumer half for draining mutations from the lock-free WAL ringbuffer.
pub struct WalConsumer<const CAP: usize = DEFAULT_WAL_BUFFER_SIZE> {
    consumer: FramedConsumer<
        alloc::sync::Arc<
            bbqueue::BBQueue<
                Inline<CAP>,
                bbqueue::traits::coordination::cas::AtomicCoord,
                bbqueue::traits::notifier::polling::Polling,
            >,
        >,
        u16,
    >,
}

/// Creates a linked pair of [`WalProducer`] and [`WalConsumer`] over an inline lock-free BipBuffer.
pub fn create_wal_channel<const CAP: usize>() -> (WalProducer<CAP>, WalConsumer<CAP>) {
    let bb: Barbacoa<CAP> = Barbacoa::new_with_storage(Inline::new());
    let producer = bb.framed_producer();
    let consumer = bb.framed_consumer();
    (WalProducer { producer }, WalConsumer { consumer })
}

impl<const CAP: usize> WalProducer<CAP> {
    /// Records a state insertion/update mutation into the WAL stream.
    ///
    /// This method is zero-allocation and lock-free.
    pub fn push_insert(
        &mut self,
        path: &str,
        value: &Value,
        revision: u64,
    ) -> Result<(), WalError> {
        let path_bytes = path.as_bytes();
        let frame_size = calculate_frame_size(path_bytes.len(), Some(value));
        if frame_size > u16::MAX as usize {
            return Err(WalError::MalformedFrame);
        }

        let mut grant = self
            .producer
            .grant(frame_size as u16)
            .map_err(|_| WalError::BufferFull)?;

        let buf = &mut *grant;
        let mut offset = 0;

        buf[offset] = OP_INSERT;
        offset += 1;

        buf[offset..offset + 8].copy_from_slice(&revision.to_le_bytes());
        offset += 8;

        let p_len = path_bytes.len() as u16;
        buf[offset..offset + 2].copy_from_slice(&p_len.to_le_bytes());
        offset += 2;

        buf[offset..offset + path_bytes.len()].copy_from_slice(path_bytes);
        offset += path_bytes.len();

        encode_value_into(value, buf, &mut offset);

        grant.commit(offset as u16);
        Ok(())
    }

    /// Records a state deletion mutation into the WAL stream.
    ///
    /// This method is zero-allocation and lock-free.
    pub fn push_delete(&mut self, path: &str, revision: u64) -> Result<(), WalError> {
        let path_bytes = path.as_bytes();
        let frame_size = calculate_frame_size(path_bytes.len(), None);
        if frame_size > u16::MAX as usize {
            return Err(WalError::MalformedFrame);
        }

        let mut grant = self
            .producer
            .grant(frame_size as u16)
            .map_err(|_| WalError::BufferFull)?;

        let buf = &mut *grant;
        let mut offset = 0;

        buf[offset] = OP_DELETE;
        offset += 1;

        buf[offset..offset + 8].copy_from_slice(&revision.to_le_bytes());
        offset += 8;

        let p_len = path_bytes.len() as u16;
        buf[offset..offset + 2].copy_from_slice(&p_len.to_le_bytes());
        offset += 2;

        buf[offset..offset + path_bytes.len()].copy_from_slice(path_bytes);
        offset += path_bytes.len();

        grant.commit(offset as u16);
        Ok(())
    }
}

impl<const CAP: usize> WalConsumer<CAP> {
    /// Attempts to read the next mutation event from the WAL ringbuffer.
    ///
    /// Returns `Ok(None)` if no events are currently available.
    pub fn pop_event(&mut self) -> Result<Option<WalEvent>, WalError> {
        let grant = match self.consumer.read() {
            Ok(g) => g,
            Err(ReadGrantError::Empty) => return Ok(None),
            Err(_) => return Err(WalError::MalformedFrame),
        };

        let buf = &*grant;
        if buf.len() < 11 {
            return Err(WalError::MalformedFrame);
        }

        let op_code = buf[0];
        let revision =
            u64::from_le_bytes(buf[1..9].try_into().map_err(|_| WalError::MalformedFrame)?);
        let path_len = u16::from_le_bytes(
            buf[9..11]
                .try_into()
                .map_err(|_| WalError::MalformedFrame)?,
        ) as usize;

        let mut offset = 11;
        if buf.len() < offset + path_len {
            return Err(WalError::MalformedFrame);
        }

        let path_str = core::str::from_utf8(&buf[offset..offset + path_len])
            .map_err(|_| WalError::InvalidPathEncoding)?
            .to_string();
        offset += path_len;

        let op = match op_code {
            OP_DELETE => WalOp::Delete { path: path_str },
            OP_INSERT => {
                let value = decode_value_from(buf, &mut offset)?;
                WalOp::Insert {
                    path: path_str,
                    value,
                }
            }
            _ => return Err(WalError::MalformedFrame),
        };

        // Explicitly release the frame back to the BipBuffer
        grant.release();

        Ok(Some(WalEvent { revision, op }))
    }

    /// Drains all available mutations from the WAL stream and applies them directly
    /// to the provided target store.
    ///
    /// Returns the number of mutations successfully applied.
    pub fn drain_to_store(&mut self, store: &mut MemStore) -> Result<usize, WalError> {
        let mut count = 0;
        while let Some(event) = self.pop_event()? {
            match event.op {
                WalOp::Insert { path, value } => {
                    let parsed = Path::parse(&path);
                    store
                        .insert(&parsed, value)
                        .map_err(|_| WalError::StoreFailure)?;
                }
                WalOp::Delete { path } => {
                    let parsed = Path::parse(&path);
                    let _ = store.remove(&parsed);
                }
            }
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfree_wal_spsc() {
        let (mut producer, mut consumer) = create_wal_channel::<2048>();

        // 1. Push events
        producer
            .push_insert("/server/tick_rate", &Value::from(128), 1)
            .unwrap();
        producer
            .push_insert("/server/motd", &Value::from("Welcome to GoldSrc-RS"), 2)
            .unwrap();
        producer
            .push_insert("/server/gravity", &Value::from(800.0), 3)
            .unwrap();
        producer.push_delete("/server/deprecated_flag", 4).unwrap();

        // 2. Consume events
        let ev1 = consumer.pop_event().unwrap().unwrap();
        assert_eq!(ev1.revision, 1);
        assert_eq!(
            ev1.op,
            WalOp::Insert {
                path: "/server/tick_rate".into(),
                value: Value::from(128),
            }
        );

        let ev2 = consumer.pop_event().unwrap().unwrap();
        assert_eq!(ev2.revision, 2);
        assert_eq!(
            ev2.op,
            WalOp::Insert {
                path: "/server/motd".into(),
                value: Value::from("Welcome to GoldSrc-RS"),
            }
        );

        let ev3 = consumer.pop_event().unwrap().unwrap();
        assert_eq!(ev3.revision, 3);
        assert_eq!(
            ev3.op,
            WalOp::Insert {
                path: "/server/gravity".into(),
                value: Value::from(800.0),
            }
        );

        let ev4 = consumer.pop_event().unwrap().unwrap();
        assert_eq!(ev4.revision, 4);
        assert_eq!(
            ev4.op,
            WalOp::Delete {
                path: "/server/deprecated_flag".into(),
            }
        );

        // Queue is now empty
        assert!(consumer.pop_event().unwrap().is_none());
    }

    #[test]
    fn test_lockfree_wal_drain_to_store() {
        let (mut producer, mut consumer) = create_wal_channel::<4096>();

        producer
            .push_insert("/cvars/mp_autoteambalance", &Value::from(true), 1)
            .unwrap();
        producer
            .push_insert("/cvars/mp_limitteams", &Value::from(2), 2)
            .unwrap();

        let mut replica = MemStore::new();
        let applied = consumer.drain_to_store(&mut replica).unwrap();
        assert_eq!(applied, 2);

        let val1 = replica
            .get(&Path::parse("/cvars/mp_autoteambalance"))
            .unwrap();
        assert_eq!(val1.value, Value::from(true));

        let val2 = replica.get(&Path::parse("/cvars/mp_limitteams")).unwrap();
        assert_eq!(val2.value, Value::from(2));
    }

    #[test]
    fn test_concurrent_producer_consumer_threads() {
        use std::thread;

        const TOTAL_EVENTS: usize = 5000;
        let (mut producer, mut consumer) = create_wal_channel::<65536>();

        let prod_handle = thread::spawn(move || {
            for i in 0..TOTAL_EVENTS {
                let path = format!("/entities/{i}/health");
                let val = Value::from(100 - (i as i64 % 50));
                while let Err(WalError::BufferFull) = producer.push_insert(&path, &val, i as u64) {
                    thread::yield_now();
                }
            }
        });

        let cons_handle = thread::spawn(move || {
            let mut received = 0;
            let mut last_rev = 0;
            while received < TOTAL_EVENTS {
                match consumer.pop_event().unwrap() {
                    Some(event) => {
                        assert_eq!(event.revision, received as u64);
                        last_rev = event.revision;
                        received += 1;
                    }
                    None => {
                        thread::yield_now();
                    }
                }
            }
            (received, last_rev)
        });

        prod_handle.join().unwrap();
        let (received, last_rev) = cons_handle.join().unwrap();
        assert_eq!(received, TOTAL_EVENTS);
        assert_eq!(last_rev, (TOTAL_EVENTS - 1) as u64);
    }
}
