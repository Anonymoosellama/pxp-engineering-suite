// SPDX-License-Identifier: LGPL-2.1-or-later
//! Property test: arbitrary property maps and objects survive the codec
//! exactly, and their encoding is canonical ("save then load is the
//! identity", master prompt testing standards).

#![allow(clippy::unwrap_used)] // test-only crate

use proptest::prelude::*;
use pxp_doc::{AppId, NewObject, Object, ObjectId, ObjectKind, Value};
use pxp_format::codec::{decode_object, decode_props, encode_object, encode_props};
use pxp_units::{Dim, Quantity};
use std::collections::BTreeMap;

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::Int),
        ".{0,40}".prop_map(Value::Text),
        (
            proptest::num::f64::NORMAL | proptest::num::f64::ZERO,
            -3i8..4,
            -1i8..2,
            -2i8..1,
            -1i8..2
        )
            .prop_map(|(v, l, m, t, a)| Value::Quantity(Quantity::from_internal(
                v,
                Dim::new(l, m, t, a)
            ))),
        any::<u128>().prop_map(|n| Value::Ref(ObjectId::from_u128(n))),
        proptest::collection::vec(any::<u128>(), 0..8)
            .prop_map(|v| Value::RefList(v.into_iter().map(ObjectId::from_u128).collect())),
        proptest::collection::vec(any::<u8>(), 0..64).prop_map(Value::Bytes),
    ]
}

fn props() -> impl Strategy<Value = BTreeMap<String, Value>> {
    proptest::collection::btree_map("[a-z_]{1,12}", value(), 0..12)
}

proptest! {
    #[test]
    fn props_round_trip_canonically(p in props()) {
        let bytes = encode_props(&p);
        let back = decode_props(&bytes).unwrap();
        prop_assert_eq!(encode_props(&back), bytes);
        prop_assert_eq!(back, p);
    }

    #[test]
    fn objects_round_trip(p in props(), id in any::<u128>(), payload in proptest::collection::vec(any::<u8>(), 0..256),
                          name in proptest::option::of("[A-Za-z0-9() ]{0,20}"), deps in proptest::collection::btree_set(any::<u128>(), 0..5)) {
        let mut spec = NewObject::new(ObjectKind::from_static("modeling.block"), AppId::from_static("pxp.modeling")).payload(payload);
        if let Some(n) = name { spec = spec.name(n); }
        for d in deps { spec = spec.dep(ObjectId::from_u128(d), "param"); }
        for (k, v) in p { spec = spec.prop(k, v); }
        let obj = Object::restore(ObjectId::from_u128(id), spec);
        let bytes = encode_object(&obj);
        let back = decode_object(&bytes).unwrap();
        prop_assert_eq!(encode_object(&back), bytes);
        prop_assert_eq!(back, obj);
    }
}
