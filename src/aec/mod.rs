//! Windows 11 Acoustic Echo Cancellation APO support.
//!
//! Gated behind the `aec` Cargo feature so that non-AEC plugins do
//! not pull in Windows 11 SDK requirements (`IApoAcousticEchoCancellation`,
//! `IApoAuxiliaryInputRT`, etc.; the AEC APO API requires Windows 11
//! 23H2 or later).
//!
//! Placeholder: the auxiliary-input wiring, reference-stream
//! handling, and `AecProcessingObject` trait are not yet
//! implemented.
