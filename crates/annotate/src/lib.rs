//! Review comments stored inside an FBX.
//!
//! A comment lives in a user-defined string property on the FBX `Model` object it
//! concerns, so the file stays a standard FBX that every application opens, and a
//! DCC that imports user properties carries the comments through a re-export.
//!
//! This crate is the whole storage side, in pure safe Rust with no ufbx, GPU or
//! window dependency — so the viewer, the `review-comments` CLI and any other tool
//! read a file's comments through exactly the same code. [`fbx`] reads and
//! losslessly patches the property in binary and ASCII files alike, and
//! [`mapping`] pairs the file's objects with the importer's scene nodes.

#![forbid(unsafe_code)]

pub mod fbx;
pub mod mapping;
