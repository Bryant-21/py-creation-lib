//! ABC back end: constant pool, instruction list, and file serializer.
//!
//! A general AVM2 emitter driven by [`crate::codegen`], with no ActionScript
//! knowledge. It is independent of `swf_native::class_abc`; an equivalence test
//! in the `swf` crate keeps the two in sync.

pub mod code;
pub mod file;
pub mod pool;
