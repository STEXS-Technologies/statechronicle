//! Allocation-free protocol primitives over borrowed bytes and caller-owned buffers.
#![no_std]
#![forbid(unsafe_code)]

pub mod codec;
pub mod integrity;
pub mod transition;
