// SPDX-License-Identifier: LGPL-2.1-or-later
//! Fuzzes the object decoder used by the recovery journal. Any input must
//! return `Ok` or `Err`, never panic, hang or over-allocate; and anything that
//! decodes must re-encode to the same bytes (the codec is canonical).
#![no_main]

use libfuzzer_sys::fuzz_target;
use pxp_format::codec::{decode_object, encode_object};

fuzz_target!(|data: &[u8]| {
    if let Ok(object) = decode_object(data) {
        assert_eq!(
            encode_object(&object),
            data,
            "decoded input must re-encode identically"
        );
    }
});
