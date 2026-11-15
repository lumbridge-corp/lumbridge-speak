//! Lumbridge Speak: the text-to-speech HTTP front.
//!
//! It serves the API of the Python front it replaced, described in `docs/API.md`. The model is behind the [`adapter::Synth`] trait.

pub mod adapter;
pub mod audio;
pub mod client;
pub mod config;
pub mod pyjson;
pub mod server;
pub mod speakable;
pub mod text;
