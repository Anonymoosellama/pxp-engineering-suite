// SPDX-License-Identifier: LGPL-2.1-or-later
//! Fuzzes the property-map decoder used for every object row of a `.pxp`
//! file. Same contract as `decode_object`: no panic, and canonical round trip.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pxp_format::codec::{decode_props, encode_props};

fuzz_target!(|data: &[u8]| {
    if let Ok(props) = decode_props(data) {
        assert_eq!(
            encode_props(&props),
            data,
            "decoded input must re-encode identically"
        );
    }
});
