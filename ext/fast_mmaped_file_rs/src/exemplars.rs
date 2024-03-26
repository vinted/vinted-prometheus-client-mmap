#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exemplar {
    // Labels (set of label names/values). Only 1 for now.
    // Value -> f64.
    // Timestamp -> uint64.
    // We have to cap the maximum size of strings.
    // From the spec:
    // The combined length of the label names and values of an Exemplar's LabelSet MUST NOT exceed 128 UTF-8 character code points. 
    // 4 bytes max per code point.
    // So, we need to allocate 128*4 = 512 bytes for the label names and values.
    pub label_name: String,
    
    pub label_value: String,
    pub value: f64,
    pub timestamp: u128,
}

use serde::{Deserialize, Serialize};

use crate::size_of;

pub const EXEMPLAR_ENTRY_MAX_SIZE_BYTES:usize = 512 + size_of::<f64>() + size_of::<u64>();

// Key -> use the old one.
// Value -> allocate EXEMPLAR_ENTRY_MAX_SIZE_BYTES. If it exceeds this, we need to return an error. Use JSON.